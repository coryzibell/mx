//! Trigger-based ambient memory: the pure logic behind `mx doors`.
//!
//! A *door* is any kv entry carrying a `triggers` list. When one of its trigger
//! phrases appears in a prompt, the door opens: a one-line fragment plus a
//! pointer to where the full fact lives. The fragment is the whole payload —
//! nothing is resolved, nothing is fetched, no graph is touched. Digging is a
//! separate, deliberate `mx kv get` the reader chooses to run.
//!
//! Everything here is IO-free so it can be tested exhaustively. The store reads,
//! the fire log and stdout live in `handlers::doors`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::kv::TriggeredRef;
use crate::triggers;

/// Distinct entries allowed to fire on a single prompt. Overflow is not
/// recorded, so a deferred door stays eligible on the next prompt.
pub const DEFAULT_BUDGET: usize = 2;

/// Longest derived fragment, in Unicode scalar values, before it is cut. An
/// authored `fragment` is printed verbatim and is never subject to this.
const DERIVED_FRAGMENT_MAX: usize = 200;

/// The kv key holding the fire log. It is both the dedup table and the
/// telemetry source: one row per (session, key, entry) fire.
pub const FIRED_KEY: &str = "doors_fired";

/// The subset of the Claude Code UserPromptSubmit payload doors reads.
///
/// Every other field is ignored rather than rejected — the hook must survive a
/// payload that grows new keys.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub prompt: String,
}

/// One door that opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FiredDoor {
    pub key: String,
    pub id: String,
    pub trigger: String,
    pub fragment: String,
    pub dig: String,
}

impl FiredDoor {
    /// The stdout line Claude Code injects as context.
    pub fn render(&self) -> String {
        format!(
            "\u{1f6aa} {} \u{2192} {} (dig: {})",
            self.trigger, self.fragment, self.dig
        )
    }
}

/// What one prompt produced: the doors that fired and how many matched but lost
/// the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Selection {
    pub fired: Vec<FiredDoor>,
    pub deferred: usize,
}

/// Remove `<channel ...>` opening tags and `</channel>` closing tags, keeping
/// the body text between them.
///
/// Matrix messages reach the hook wrapped in a channel block whose ATTRIBUTES
/// carry the sender's identity: `user="carmel"`, `room_name="delta"`. Left in
/// place, a `carmel` door would fire on every message in that room regardless of
/// what was said. The body is what a person actually wrote, so it is the only
/// part that may open a door.
///
/// The scan is quote-aware: an unbalanced `>` inside an attribute value does not
/// end the tag.
pub fn strip_channel_tags(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    let bytes = prompt.as_bytes();
    let mut i = 0;
    while i < prompt.len() {
        let rest = &prompt[i..];
        if let Some(after) = rest.strip_prefix("</channel>") {
            out.push(' ');
            i += rest.len() - after.len();
            continue;
        }
        if rest.starts_with("<channel")
            && rest[8..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c == '>')
        {
            let mut j = i + 8;
            let mut in_quote = false;
            while j < prompt.len() {
                match bytes[j] {
                    b'"' => in_quote = !in_quote,
                    b'>' if !in_quote => {
                        j += 1;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            out.push(' ');
            i = j;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The one-line fragment for an entry.
///
/// An authored fragment is returned verbatim, however long. Otherwise the
/// entry's `value` up to its first newline, trimmed, cut at
/// `DERIVED_FRAGMENT_MAX` scalar values with an ellipsis. Cutting by `chars`
/// keeps the boundary valid, so a multi-byte character is never split.
pub fn derive_fragment(value: &str, fragment: Option<&str>) -> String {
    if let Some(f) = fragment {
        let f = f.trim();
        if !f.is_empty() {
            return f.to_string();
        }
    }
    let first_line = value.split('\n').next().unwrap_or("").trim();
    if first_line.chars().count() > DERIVED_FRAGMENT_MAX {
        let kept: String = first_line.chars().take(DERIVED_FRAGMENT_MAX - 1).collect();
        format!("{}\u{2026}", kept)
    } else {
        first_line.to_string()
    }
}

/// The pointer a reader follows to the full fact: always `<key>/kv-<id>`, plus
/// the kn- link when the entry carries one. Printed, never resolved.
pub fn dig_pointer(key: &str, id: &str, memory: Option<&str>) -> String {
    match memory {
        Some(m) if !m.trim().is_empty() => format!("{}/kv-{}, {}", key, id, m.trim()),
        _ => format!("{}/kv-{}", key, id),
    }
}

/// Decide which doors open for one prompt.
///
/// `already_fired` holds the `(key, entry id)` pairs this session has seen, and
/// `fire_counts` the all-time count per pair. Ordering is fewest all-time fires
/// first, then oldest `ts` — a door nobody has opened yet outranks a familiar
/// one, so the budget goes to what the reader is least likely to already hold.
pub fn select(
    prompt: &str,
    candidates: &[TriggeredRef<'_>],
    already_fired: &HashSet<(String, String)>,
    fire_counts: &HashMap<(String, String), u64>,
    budget: usize,
) -> Selection {
    let text = strip_channel_tags(prompt);
    // Stemming OFF: doors are proper nouns, and the English stemmer folds
    // Tagalog "ayos" onto "ayo". Morphology is the enemy here.
    let message_tokens = triggers::tokens(&text, false);
    if message_tokens.is_empty() {
        return Selection {
            fired: Vec::new(),
            deferred: 0,
        };
    }

    let mut matched: Vec<(u64, &str, FiredDoor)> = Vec::new();
    for cand in candidates {
        let pair = (cand.key.to_string(), cand.id.to_string());
        if already_fired.contains(&pair) {
            continue;
        }
        let hits = triggers::match_triggers(&message_tokens, cand.triggers, false);
        let Some(trigger) = hits.into_iter().next() else {
            continue;
        };
        let count = fire_counts.get(&pair).copied().unwrap_or(0);
        matched.push((
            count,
            cand.ts,
            FiredDoor {
                key: pair.0,
                id: pair.1,
                trigger,
                fragment: derive_fragment(cand.value, cand.fragment),
                dig: dig_pointer(cand.key, cand.id, cand.memory),
            },
        ));
    }

    matched.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.key.cmp(&b.2.key))
            .then_with(|| a.2.id.cmp(&b.2.id))
    });

    let total = matched.len();
    let fired: Vec<FiredDoor> = matched.into_iter().take(budget).map(|m| m.2).collect();
    let deferred = total - fired.len();
    Selection { fired, deferred }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = concat!(
        r#"<channel source="matrix" chat_id="!r:s" message_id="$e" user="carmel" "#,
        r#"user_id="@j:s" room_name="delta">"#,
        "\ngood morning konkon\n</channel>"
    );

    fn cand<'a>(
        key: &'a str,
        id: &'a str,
        value: &'a str,
        ts: &'a str,
        triggers: &'a [String],
    ) -> TriggeredRef<'a> {
        TriggeredRef {
            key,
            id,
            value,
            ts,
            triggers,
            fragment: None,
            memory: None,
        }
    }

    // ---- strip_channel_tags ----

    #[test]
    fn strip_channel_removes_tag_and_keeps_body() {
        let out = strip_channel_tags(CHANNEL);
        assert!(out.contains("good morning konkon"));
        assert!(
            !out.contains("carmel"),
            "attributes must not survive: {out}"
        );
        assert!(!out.contains("delta"));
        assert!(!out.contains("channel"));
    }

    #[test]
    fn strip_channel_attribute_alone_does_not_fire_a_door() {
        let trig = vec!["carmel".to_string()];
        let cands = [cand("facts", "aaa", "x", "2026-01-01T00:00:00Z", &trig)];
        let sel = select(CHANNEL, &cands, &HashSet::new(), &HashMap::new(), 2);
        assert!(
            sel.fired.is_empty(),
            "user=\"carmel\" must not open a carmel door"
        );
    }

    #[test]
    fn strip_channel_tolerates_gt_inside_an_attribute() {
        let out = strip_channel_tags(r#"<channel room_name="a > b">hi</channel>"#);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn strip_channel_leaves_ordinary_angle_brackets() {
        let out = strip_channel_tags("if a < b and c > d");
        assert_eq!(out, "if a < b and c > d");
    }

    // ---- fragment derivation ----

    #[test]
    fn authored_fragment_is_verbatim_however_long() {
        let long = "z".repeat(400);
        assert_eq!(derive_fragment("value", Some(&long)), long);
    }

    #[test]
    fn derived_fragment_is_the_first_line() {
        assert_eq!(
            derive_fragment("  first line  \nsecond line", None),
            "first line"
        );
    }

    #[test]
    fn derived_fragment_cuts_on_a_char_boundary() {
        // A multi-byte character sits exactly at the cut point.
        let mut line = "a".repeat(198);
        line.push('é');
        line.push_str(&"b".repeat(60));
        let out = derive_fragment(&line, None);
        assert_eq!(out.chars().count(), DERIVED_FRAGMENT_MAX);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.contains('é'), "the boundary char must survive intact");
    }

    #[test]
    fn empty_authored_fragment_falls_back_to_first_line() {
        assert_eq!(derive_fragment("the value", Some("   ")), "the value");
    }

    // ---- dig pointer ----

    #[test]
    fn dig_pointer_shapes() {
        assert_eq!(dig_pointer("doors", "2gRwr9", None), "doors/kv-2gRwr9");
        assert_eq!(
            dig_pointer("doors", "2gRwr9", Some("kn-02e73234")),
            "doors/kv-2gRwr9, kn-02e73234"
        );
    }

    // ---- matching ----

    #[test]
    fn konkon_fires_from_inside_a_channel_block() {
        let trig = vec!["konkon".to_string()];
        let cands = [cand(
            "facts",
            "3gtR1J",
            "Carmel's everyday word for Q, the fox-sound.",
            "2026-01-01T00:00:00Z",
            &trig,
        )];
        let sel = select(CHANNEL, &cands, &HashSet::new(), &HashMap::new(), 2);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].trigger, "konkon");
        assert_eq!(sel.fired[0].dig, "facts/kv-3gtR1J");
        assert_eq!(
            sel.fired[0].render(),
            "\u{1f6aa} konkon \u{2192} Carmel's everyday word for Q, the fox-sound. (dig: facts/kv-3gtR1J)"
        );
    }

    #[test]
    fn already_fired_entry_is_skipped() {
        let trig = vec!["konkon".to_string()];
        let cands = [cand("facts", "aaa", "v", "2026-01-01T00:00:00Z", &trig)];
        let fired: HashSet<(String, String)> = [("facts".to_string(), "aaa".to_string())]
            .into_iter()
            .collect();
        let sel = select("hi konkon", &cands, &fired, &HashMap::new(), 2);
        assert!(sel.fired.is_empty());
        assert_eq!(
            sel.deferred, 0,
            "a deduped door is not deferred, it is done"
        );
    }

    #[test]
    fn dedup_tuple_includes_the_key() {
        // Two entries in DIFFERENT keys sharing one 6-char id. Firing one must
        // not suppress the other.
        let trig = vec!["konkon".to_string()];
        let cands = [
            cand("facts", "same01", "a", "2026-01-01T00:00:00Z", &trig),
            cand("doors", "same01", "b", "2026-01-02T00:00:00Z", &trig),
        ];
        let fired: HashSet<(String, String)> = [("facts".to_string(), "same01".to_string())]
            .into_iter()
            .collect();
        let sel = select("konkon", &cands, &fired, &HashMap::new(), 2);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].key, "doors");
    }

    #[test]
    fn budget_overflow_defers_the_most_fired() {
        let trig = vec!["konkon".to_string()];
        let cands = [
            cand("d", "aaa", "a", "2026-01-01T00:00:00Z", &trig),
            cand("d", "bbb", "b", "2026-01-01T00:00:00Z", &trig),
            cand("d", "ccc", "c", "2026-01-01T00:00:00Z", &trig),
        ];
        let counts: HashMap<(String, String), u64> = [
            (("d".to_string(), "aaa".to_string()), 7),
            (("d".to_string(), "bbb".to_string()), 1),
            (("d".to_string(), "ccc".to_string()), 0),
        ]
        .into_iter()
        .collect();
        let sel = select("konkon", &cands, &HashSet::new(), &counts, 2);
        assert_eq!(sel.deferred, 1);
        let ids: Vec<&str> = sel.fired.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["ccc", "bbb"], "least-fired first");
    }

    #[test]
    fn tie_on_fire_count_breaks_to_oldest_ts() {
        let trig = vec!["konkon".to_string()];
        let cands = [
            cand("d", "new", "n", "2026-06-01T00:00:00Z", &trig),
            cand("d", "old", "o", "2020-01-01T00:00:00Z", &trig),
        ];
        let sel = select("konkon", &cands, &HashSet::new(), &HashMap::new(), 1);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].id, "old");
    }

    #[test]
    fn no_match_is_an_empty_selection() {
        let trig = vec!["konkon".to_string()];
        let cands = [cand("d", "aaa", "a", "2026-01-01T00:00:00Z", &trig)];
        let sel = select("hello there", &cands, &HashSet::new(), &HashMap::new(), 2);
        assert!(sel.fired.is_empty());
        assert_eq!(sel.deferred, 0);
    }

    #[test]
    fn stemming_is_off_so_tagalog_ayos_does_not_open_the_ayo_door() {
        let trig = vec!["ayo-".to_string()];
        let cands = [cand(
            "d",
            "aaa",
            "shell alias",
            "2026-01-01T00:00:00Z",
            &trig,
        )];
        assert!(
            select("ayos lang", &cands, &HashSet::new(), &HashMap::new(), 2)
                .fired
                .is_empty()
        );
        assert_eq!(
            select(
                "switched to ayo-mirage",
                &cands,
                &HashSet::new(),
                &HashMap::new(),
                2
            )
            .fired
            .len(),
            1
        );
    }
}
