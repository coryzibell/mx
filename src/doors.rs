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
    #[serde(default, deserialize_with = "lenient_string")]
    pub session_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub prompt: String,
}

/// Accept a JSON string, number, or null where a string is expected.
///
/// Claude Code sends `session_id` as a string today. If that ever arrives as a
/// number, a strict `String` field would fail the WHOLE payload and the hook
/// would go silent for that prompt with no door and no clue why. Coercing is
/// strictly better than losing the turn: a numeric id still dedups correctly,
/// it just stringifies first.
fn lenient_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
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
/// carry the sender's identity: `user="tester"`, `room_name="delta"`. Left in
/// place, a `tester` door would fire on every message in that room regardless of
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
            let mut closed = false;
            while j < prompt.len() {
                match bytes[j] {
                    b'"' => in_quote = !in_quote,
                    b'>' if !in_quote => {
                        j += 1;
                        closed = true;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if closed {
                out.push(' ');
                i = j;
                continue;
            }
            // No closing bracket: this is not a real channel tag (a stray
            // "<channel" someone typed, or an unbalanced quote). Swallowing to
            // end-of-string would silently cost every door for the turn, so
            // treat the remainder as ordinary body text and stop stripping.
            out.push_str(&prompt[i..]);
            break;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Harness block names matched exactly. `local-command-*` is handled apart
/// because its suffix varies.
const HARNESS_BLOCKS: [&str; 2] = ["task-notification", "system-reminder"];
const LOCAL_COMMAND_PREFIX: &str = "local-command-";

/// The harness tag name `s` begins with, if any. A `local-command-` name takes
/// the longest `[a-z0-9-]` suffix, so the caller's `>` check is what rejects
/// `local-command-stdoutX`.
fn harness_name(s: &str) -> Option<&str> {
    if let Some(name) = HARNESS_BLOCKS.into_iter().find(|n| s.starts_with(n)) {
        return Some(name);
    }
    let suffix = s.strip_prefix(LOCAL_COMMAND_PREFIX)?;
    let n = suffix
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        .count();
    (n > 0).then(|| &s[..LOCAL_COMMAND_PREFIX.len() + n])
}

enum TagKind {
    Open,
    SelfClosing,
    Close,
}

struct Tag<'a> {
    start: usize,
    end: usize,
    kind: TagKind,
    name: &'a str,
}

/// The bare harness tag at the start of `s`: exactly `<name>`, `<name/>` or
/// `</name>`. Attributes, whitespace and case variants are not harness tags.
fn bare_tag(s: &str) -> Option<(TagKind, &str, usize)> {
    let s = s.strip_prefix('<')?;
    let (closing, s) = match s.strip_prefix('/') {
        Some(s) => (true, s),
        None => (false, s),
    };
    let name = harness_name(s)?;
    let after = &s[name.len()..];
    let lead = 1 + usize::from(closing) + name.len();
    if after.starts_with('>') {
        let kind = if closing {
            TagKind::Close
        } else {
            TagKind::Open
        };
        Some((kind, name, lead + 1))
    } else if !closing && after.starts_with("/>") {
        Some((TagKind::SelfClosing, name, lead + 2))
    } else {
        None
    }
}

/// Remove harness-injected blocks, body included: `<task-notification>`,
/// `<system-reminder>` and every `<local-command-*>` (closed by the same name).
///
/// Claude Code puts these in the prompt the hook receives, but the human never
/// typed them. A trigger word inside a subagent report or a CLI echo would open
/// a door, burn its per-session dedup slot and eat the prompt's budget.
///
/// Only bare tags count (the harness never writes attributes), so prose that
/// mentions a tag name is left alone. Openers pair with closers by depth, per
/// name. An opener nothing balances (a report quoting its own tag) ends at the
/// first `</name>` after it; with no `</name>` after it at all, only the opener
/// goes. Each removed block or stray tag becomes one space, so the text on
/// either side stays separate tokens.
pub fn strip_harness_blocks(prompt: &str) -> String {
    let tags: Vec<Tag<'_>> = prompt
        .match_indices('<')
        .filter_map(|(start, _)| {
            let (kind, name, len) = bare_tag(&prompt[start..])?;
            Some(Tag {
                start,
                end: start + len,
                kind,
                name,
            })
        })
        .collect();

    // block_end[i]: where the block opened by tag i ends, for openers that
    // have one. Depth-matched closers first, then the first-closer fallback.
    let mut block_end: Vec<Option<usize>> = vec![None; tags.len()];
    let mut open: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, tag) in tags.iter().enumerate() {
        match tag.kind {
            TagKind::Open => open.entry(tag.name).or_default().push(i),
            TagKind::Close => {
                if let Some(o) = open.get_mut(tag.name).and_then(Vec::pop) {
                    block_end[o] = Some(tag.end);
                }
            }
            TagKind::SelfClosing => {}
        }
    }
    let mut next_close: HashMap<&str, usize> = HashMap::new();
    for (i, tag) in tags.iter().enumerate().rev() {
        match tag.kind {
            TagKind::Close => {
                next_close.insert(tag.name, tag.end);
            }
            TagKind::Open if block_end[i].is_none() => {
                block_end[i] = next_close.get(tag.name).copied();
            }
            _ => {}
        }
    }

    let mut out = String::with_capacity(prompt.len());
    let mut pos = 0;
    for (i, tag) in tags.iter().enumerate() {
        if tag.start < pos {
            continue;
        }
        out.push_str(&prompt[pos..tag.start]);
        out.push(' ');
        pos = match tag.kind {
            TagKind::Open => block_end[i].unwrap_or(tag.end),
            TagKind::SelfClosing | TagKind::Close => tag.end,
        };
    }
    out.push_str(&prompt[pos..]);
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

/// Shortest single-token trigger that is not flagged as dangerously broad.
const MIN_TRIGGER_TOKEN_CHARS: usize = 3;

/// What a trigger will actually match on, and whether that is a problem.
///
/// Matching tokenizes on runs of non-alphanumeric characters, so a name carrying
/// punctuation collapses in ways the author does not expect: `c++`, `c#` and
/// `F#` all match as the bare token `c` or `f`, and `🦊` matches as nothing at
/// all. A trigger that matches nothing is a dead door — it sits in the audit
/// view at `fires=0` forever and looks like a door nobody has said the word for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerVerdict {
    /// No alphanumeric content survives tokenization; it can never fire.
    Dead,
    /// Fires, but as a single very short token, so it will fire constantly.
    Broad { matched_as: String },
    /// Fires as `matched_as`, which may still differ from what was typed.
    Fine { matched_as: String },
}

/// The token sequence a trigger actually matches on, space-joined. This is what
/// the reader should be shown, because it is what the matcher sees — `ayo-`
/// matches as `ayo`, and `gerf_slips` as `gerf slips`.
pub fn matched_form(trigger: &str) -> String {
    triggers::tokens(trigger, false).join(" ")
}

/// Judge an authored trigger before it is stored.
pub fn inspect_trigger(trigger: &str) -> TriggerVerdict {
    let toks = triggers::tokens(trigger, false);
    if toks.is_empty() {
        return TriggerVerdict::Dead;
    }
    let matched_as = toks.join(" ");
    if toks.len() == 1 && toks[0].chars().count() < MIN_TRIGGER_TOKEN_CHARS {
        TriggerVerdict::Broad { matched_as }
    } else {
        TriggerVerdict::Fine { matched_as }
    }
}

/// How specific a trigger is: how many message tokens it spans, then how many
/// characters. A longer span is a more precise statement about the message.
fn specificity(trigger: &str) -> (usize, usize) {
    (
        triggers::tokens(trigger, false).len(),
        trigger.chars().count(),
    )
}

/// Decide which doors open for one prompt.
///
/// `already_fired` holds the `(key, entry id)` pairs this session has seen, and
/// `fire_counts` the all-time count per pair. Ordering is fewest all-time fires
/// first, then oldest `ts` — a door nobody has opened yet outranks a familiar
/// one, so the budget goes to what the reader is least likely to already hold.
///
/// Only human text is matched: harness blocks are removed whole first, then
/// channel tags are stripped down to their body.
pub fn select(
    prompt: &str,
    candidates: &[TriggeredRef<'_>],
    already_fired: &HashSet<(String, String)>,
    fire_counts: &HashMap<(String, String), u64>,
    budget: usize,
) -> Selection {
    let text = strip_channel_tags(&strip_harness_blocks(prompt));
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
        // Report the MOST SPECIFIC trigger, not whichever was stored first.
        // An entry carrying both `gerf` and `gerf_slips` should say
        // `gerf_slips` fired on "gerf_slips is permanent" — the longer match
        // covers more of the message and is what the reader needs to see.
        // Specificity is token count first, then characters; ties keep stored
        // order so output stays deterministic.
        let Some(trigger) = hits.into_iter().reduce(|best, t| {
            if specificity(&t) > specificity(&best) {
                t
            } else {
                best
            }
        }) else {
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
        r#"<channel source="matrix" chat_id="!r:s" message_id="$e" user="tester" "#,
        r#"user_id="@j:s" room_name="delta">"#,
        "\ngood morning widget\n</channel>"
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
        assert!(out.contains("good morning widget"));
        assert!(
            !out.contains("tester"),
            "attributes must not survive: {out}"
        );
        assert!(!out.contains("delta"));
        assert!(!out.contains("channel"));
    }

    #[test]
    fn strip_channel_attribute_alone_does_not_fire_a_door() {
        let trig = vec!["tester".to_string()];
        let cands = [cand("facts", "aaa", "x", "2026-01-01T00:00:00Z", &trig)];
        let sel = select(CHANNEL, &cands, &HashSet::new(), &HashMap::new(), 2);
        assert!(
            sel.fired.is_empty(),
            "user=\"tester\" must not open a tester door"
        );
    }

    #[test]
    fn strip_channel_tolerates_gt_inside_an_attribute() {
        let out = strip_channel_tags(r#"<channel room_name="a > b">hi</channel>"#);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn unterminated_channel_tag_does_not_swallow_the_turn() {
        // A stray "<channel" with no closing bracket must not eat the rest of
        // the prompt -- that would cost every door for the turn.
        let out = strip_channel_tags("<channel source=\"matrix\" oops widget is here");
        assert!(out.contains("widget"), "body survived: {out}");

        // Same with an unbalanced quote, which is how the scan runs off the end.
        let out = strip_channel_tags("<channel user=\"unclosed widget");
        assert!(out.contains("widget"), "body survived: {out}");
    }

    #[test]
    fn unterminated_channel_tag_still_fires_doors() {
        let trig = vec!["widget".to_string()];
        let cands = [cand("facts", "aaa", "v", "2026-01-01T00:00:00Z", &trig)];
        let sel = select(
            "<channel user=\"unclosed good morning widget",
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
        assert_eq!(sel.fired.len(), 1, "a malformed tag must not lose the door");
    }

    #[test]
    fn most_specific_trigger_is_reported() {
        // Stored order puts the SHORT trigger first, so first-match would
        // report `gerf` for a message that actually said `gerf_slips`.
        let trig = vec!["gerf".to_string(), "gerf_slips".to_string()];
        let cands = [cand(
            "doors",
            "aaa",
            "Gerf is GEOFF",
            "2026-01-01T00:00:00Z",
            &trig,
        )];
        let sel = select(
            "gerf_slips is permanent",
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
        assert_eq!(sel.fired[0].trigger, "gerf_slips");

        // And a message with only the short form still reports the short form.
        let sel = select("ask gerf", &cands, &HashSet::new(), &HashMap::new(), 2);
        assert_eq!(sel.fired[0].trigger, "gerf");
    }

    #[test]
    fn dead_triggers_are_recognised() {
        // No alphanumeric content survives tokenization.
        assert_eq!(inspect_trigger("\u{1f98a}"), TriggerVerdict::Dead);
        assert_eq!(inspect_trigger("!!!"), TriggerVerdict::Dead);
        assert_eq!(inspect_trigger("---"), TriggerVerdict::Dead);
    }

    #[test]
    fn punctuation_names_collapse_to_a_broad_single_token() {
        // The whole point: the author types a language name, the matcher sees a
        // single letter.
        for (raw, becomes) in [("c++", "c"), ("c#", "c"), ("F#", "f")] {
            assert_eq!(
                inspect_trigger(raw),
                TriggerVerdict::Broad {
                    matched_as: becomes.to_string()
                },
                "{raw} must be flagged"
            );
        }
    }

    #[test]
    fn ordinary_triggers_are_fine_and_report_what_they_match() {
        assert_eq!(
            inspect_trigger("widget"),
            TriggerVerdict::Fine {
                matched_as: "widget".to_string()
            }
        );
        // Fine, but the matched form differs from the stored text -- the caller
        // surfaces that so `ayo-` is not a surprise.
        assert_eq!(
            inspect_trigger("ayo-"),
            TriggerVerdict::Fine {
                matched_as: "ayo".to_string()
            }
        );
        assert_eq!(matched_form("gerf_slips"), "gerf slips");
        assert_eq!(matched_form(".NET"), "net");
    }

    #[test]
    fn lenient_string_accepts_a_numeric_session_id() {
        // A strict String field would reject the whole payload and the hook
        // would go silent for that prompt.
        let input: HookInput =
            serde_json::from_str(r#"{"session_id": 12345, "prompt": "hi widget"}"#).unwrap();
        assert_eq!(input.session_id, "12345");
        assert_eq!(input.prompt, "hi widget");
    }

    #[test]
    fn hook_input_tolerates_missing_null_and_unknown_fields() {
        let input: HookInput = serde_json::from_str(
            r#"{"prompt": "hi", "session_id": null, "something_new": {"a": 1}}"#,
        )
        .unwrap();
        assert_eq!(input.session_id, "");
        assert_eq!(input.prompt, "hi");

        let bare: HookInput = serde_json::from_str("{}").unwrap();
        assert_eq!(bare.prompt, "");
    }

    #[test]
    fn specificity_prefers_more_tokens_then_more_characters() {
        assert!(specificity("kon kon") > specificity("widget"));
        assert!(specificity("gerf_slips") > specificity("gerf"));
        // Equal triggers tie, so stored order decides and output stays stable.
        assert_eq!(specificity("alpha"), specificity("bravo"));
    }

    #[test]
    fn strip_channel_leaves_ordinary_angle_brackets() {
        let out = strip_channel_tags("if a < b and c > d");
        assert_eq!(out, "if a < b and c > d");
    }

    // ---- strip_harness_blocks ----

    /// The triggers that opened a door on `prompt`, one candidate per trigger.
    fn fired_on(prompt: &str, triggers: &[&str]) -> Vec<String> {
        let owned: Vec<Vec<String>> = triggers.iter().map(|t| vec![t.to_string()]).collect();
        let cands: Vec<TriggeredRef<'_>> = owned
            .iter()
            .enumerate()
            .map(|(n, t)| TriggeredRef {
                key: "d",
                id: triggers[n],
                value: "v",
                ts: "2026-01-01T00:00:00Z",
                triggers: t,
                fragment: None,
                memory: None,
            })
            .collect();
        let mut fired: Vec<String> = select(
            prompt,
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            triggers.len(),
        )
        .fired
        .into_iter()
        .map(|f| f.trigger)
        .collect();
        fired.sort();
        fired
    }

    #[test]
    fn harness_blocks_are_removed_with_their_body() {
        for (prompt, gone) in [
            (
                "a <task-notification>\n<status>done</status> secret\n</task-notification> b",
                "secret",
            ),
            ("a <system-reminder>secret</system-reminder> b", "secret"),
            (
                "a <local-command-stdout>secret</local-command-stdout> b",
                "secret",
            ),
            (
                "a <local-command-stderr>secret</local-command-stderr> b",
                "secret",
            ),
            (
                "a <local-command-caveat>secret</local-command-caveat> b",
                "secret",
            ),
        ] {
            let out = strip_harness_blocks(prompt);
            assert!(!out.contains(gone), "body must go: {out:?}");
            assert!(!out.contains('<'), "tags must go: {out:?}");
            assert_eq!(triggers::tokens(&out, false), vec!["a", "b"], "{out:?}");
        }
    }

    #[test]
    fn trigger_inside_a_harness_block_does_not_fire_but_human_text_does() {
        let prompt = concat!(
            "<task-notification>agent quoted widget and gizmo</task-notification>",
            "<system-reminder>gizmo widget sprocket</system-reminder>",
            "<local-command-stdout>sprocket</local-command-stdout>",
            "\nwhat about the widget?"
        );
        assert_eq!(
            fired_on(prompt, &["widget", "gizmo", "sprocket"]),
            vec!["widget"]
        );
    }

    #[test]
    fn tag_name_trigger_does_not_fire_off_the_tag() {
        assert!(
            fired_on(
                "<system-reminder>note</system-reminder> hi",
                &["system-reminder", "task-notification"]
            )
            .is_empty()
        );
        // A stray closer and a self-closing tag are tags too.
        assert!(
            fired_on(
                "hi </system-reminder> <system-reminder/>",
                &["system-reminder"]
            )
            .is_empty()
        );
    }

    #[test]
    fn near_miss_names_are_not_harness_tags() {
        let prompt = "<system-reminderz>keep</system-reminderz> <System-Reminder>keep2</System-Reminder> <local-command->keep3</local-command->";
        let out = strip_harness_blocks(prompt);
        assert_eq!(out, prompt, "only exact lowercase names are stripped");
    }

    #[test]
    fn unclosed_opener_keeps_the_rest_as_text() {
        let out = strip_harness_blocks("hi <system-reminder oops widget");
        assert!(out.contains("widget"), "{out:?}");
        assert_eq!(
            fired_on("hi <system-reminder oops widget", &["widget"]),
            vec!["widget"]
        );
        // A later real block is still removed.
        assert_eq!(
            fired_on(
                "hi <system-reminder oops widget <system-reminder>gizmo</system-reminder>",
                &["widget", "gizmo"]
            ),
            vec!["widget"]
        );
    }

    #[test]
    fn only_bare_tags_are_harness_tags() {
        for prompt in [
            "<system-reminder note=\"x\">widget</task-notification-not>",
            "<system-reminder >widget",
            "<System-Reminder>widget</System-Reminder>",
            "<local-command-é>widget</local-command-é>",
            "<task-notification id=\"1\"/>widget",
        ] {
            assert_eq!(strip_harness_blocks(prompt), prompt);
            assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"], "{prompt:?}");
        }
    }

    #[test]
    fn prose_mention_of_a_tag_keeps_the_human_text() {
        let prompt = "the <system-reminder thing keeps opening keurig -> please fix";
        assert_eq!(strip_harness_blocks(prompt), prompt);
        assert_eq!(fired_on(prompt, &["keurig"]), vec!["keurig"]);
    }

    #[test]
    fn attributed_or_split_openers_are_literal_text() {
        let prompt = "<system-reminder a=\">\" b=\"<\">widget</system-reminder>\n<task-notification\n  id=\"1\"\n>widget</task-notification> gizmo";
        assert_eq!(
            strip_harness_blocks(prompt),
            "<system-reminder a=\">\" b=\"<\">widget \n<task-notification\n  id=\"1\"\n>widget  gizmo"
        );
        assert_eq!(
            fired_on(prompt, &["widget", "gizmo"]),
            vec!["gizmo", "widget"]
        );
        assert_eq!(
            fired_on("<system-\nreminder>widget</system-\nreminder>", &["widget"]),
            vec!["widget"]
        );
    }

    #[test]
    fn opener_with_no_closing_tag_loses_only_the_tag() {
        let prompt = "<system-reminder>so about the widget";
        let out = strip_harness_blocks(prompt);
        assert_eq!(out, " so about the widget");
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn report_quoting_its_own_opener_is_removed_through_the_first_closer() {
        let prompt = concat!(
            "<task-notification>\n<status>completed</status>\n",
            "<result>Ran kv update on keurig, then stripped `<task-notification>` blocks.</result>\n",
            "</task-notification>\nhi"
        );
        assert_eq!(strip_harness_blocks(prompt), " \nhi");
        assert!(fired_on(prompt, &["keurig"]).is_empty());
        // First closer, not last: human text between two blocks survives.
        let prompt = "<system-reminder>a <system-reminder>b</system-reminder> widget <system-reminder>c</system-reminder>";
        assert_eq!(strip_harness_blocks(prompt), "  widget  ");
    }

    #[test]
    fn known_edge_quoted_lone_closer_ends_the_block_early() {
        // Waived: a body quoting its own closer ends there, so the tail leaks.
        // It leaks agent text and never eats human text.
        let prompt =
            "<task-notification>the `</task-notification>` closer, sprocket</task-notification>";
        assert_eq!(strip_harness_blocks(prompt), " ` closer, sprocket ");
        assert_eq!(fired_on(prompt, &["sprocket"]), vec!["sprocket"]);
    }

    #[test]
    fn stray_closer_is_removed_and_text_survives() {
        let prompt = "the widget</task-notification>works";
        assert_eq!(strip_harness_blocks(prompt), "the widget works");
        assert_eq!(
            fired_on(prompt, &["widget", "works"]),
            vec!["widget", "works"]
        );
    }

    #[test]
    fn self_closing_tag_is_removed_alone() {
        let prompt = "the<system-reminder/>widget <local-command-caveat/>works";
        assert_eq!(strip_harness_blocks(prompt), "the widget  works");
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn self_closing_tag_does_not_open_a_block() {
        let prompt = "<system-reminder/> widget </system-reminder> gizmo";
        assert_eq!(strip_harness_blocks(prompt), "  widget   gizmo");
        assert_eq!(
            fired_on(prompt, &["widget", "gizmo"]),
            vec!["gizmo", "widget"]
        );
    }

    #[test]
    fn nested_same_name_blocks_go_entirely() {
        let prompt =
            "<system-reminder>a<system-reminder>b</system-reminder>c</system-reminder> widget";
        assert_eq!(strip_harness_blocks(prompt), "  widget");
        assert!(fired_on(prompt, &["c"]).is_empty());
    }

    #[test]
    fn nested_different_name_goes_with_the_outer_body() {
        let prompt = concat!(
            "<task-notification>report <system-reminder>inner</system-reminder> ",
            "<local-command-stdout>echo</local-command-stdout> tail</task-notification> widget"
        );
        assert_eq!(strip_harness_blocks(prompt), "  widget");
        assert_eq!(
            fired_on(prompt, &["widget", "inner", "echo", "tail"]),
            vec!["widget"]
        );
    }

    #[test]
    fn adjacent_blocks_each_go() {
        let prompt = "<system-reminder>a</system-reminder><system-reminder>b</system-reminder><task-notification>c</task-notification>widget";
        assert_eq!(strip_harness_blocks(prompt), "   widget");
    }

    #[test]
    fn removed_block_leaves_a_token_boundary() {
        let out = strip_harness_blocks("foo<system-reminder>x</system-reminder>bar");
        assert_eq!(out, "foo bar");
        assert_eq!(triggers::tokens(&out, false), vec!["foo", "bar"]);
        assert!(fired_on("foo<system-reminder>x</system-reminder>bar", &["foobar"]).is_empty());
    }

    #[test]
    fn mismatched_local_command_close_does_not_swallow_the_turn() {
        let prompt = "<local-command-stdout>out</local-command-stderr> widget";
        assert_eq!(strip_harness_blocks(prompt), " out  widget");
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn deep_nesting_balanced() {
        let n = 5_000;
        let prompt = format!(
            "{}widget{} gizmo",
            "<system-reminder>".repeat(n),
            "</system-reminder>".repeat(n)
        );
        assert_eq!(fired_on(&prompt, &["widget", "gizmo"]), vec!["gizmo"]);
    }

    #[test]
    fn multibyte_hugging_tags() {
        let prompt = "é<system-reminder>🦊</system-reminder>ß<local-command-stdout>\u{200b}</local-command-stdout>🦊widget";
        assert_eq!(strip_harness_blocks(prompt), "é ß 🦊widget");
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
        for literal in [
            "<system-reminder",
            "<system-reminder/",
            "<local-command-",
            "",
        ] {
            assert_eq!(strip_harness_blocks(literal), literal);
        }
        assert!(
            fired_on(
                "<system-reminder>widget</system-reminder><task-notification>widget</task-notification>",
                &["widget"]
            )
            .is_empty()
        );
    }

    #[test]
    fn fuzz_multibyte_around_tags_never_panics() {
        const PIECES: [&str; 22] = [
            "<",
            "</",
            ">",
            "/>",
            "\"",
            " ",
            "\n",
            "x",
            "é",
            "🦊",
            "\u{200b}",
            "ß",
            "system-reminder",
            "task-notification",
            "local-command-stdout",
            "local-command-",
            "local-command-é",
            "<system-reminder",
            "</system-reminder>",
            "<channel",
            "</channel>",
            "<task-notification>",
        ];
        let mut seed: u64 = 0x5eed_b0bb;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        for _ in 0..20_000 {
            let len = next() % 24;
            let s: String = (0..len)
                .map(|_| PIECES[(next() % PIECES.len() as u64) as usize])
                .collect();
            let _ = strip_channel_tags(&strip_harness_blocks(&s));
            let _ = fired_on(&s, &["x", "system-reminder"]);
        }
    }

    #[test]
    fn many_unbalanced_openers_stay_linear() {
        use std::time::{Duration, Instant};
        for unit in ["<system-reminder>x ", "<local-command-stdout>x "] {
            let prompt = format!("{} widget", unit.repeat(20_000));
            let t = Instant::now();
            let out = strip_harness_blocks(&prompt);
            let took = t.elapsed();
            assert!(out.ends_with(" widget"));
            assert!(
                took < Duration::from_millis(500),
                "{} KB of {unit:?} took {took:?}",
                prompt.len() / 1024
            );
        }
    }

    #[test]
    fn channel_body_still_fires_next_to_a_harness_block() {
        assert_eq!(fired_on(CHANNEL, &["widget", "tester"]), vec!["widget"]);
        let prompt = format!(
            "<system-reminder>gizmo tester</system-reminder>{CHANNEL}<task-notification>gizmo</task-notification>"
        );
        assert_eq!(
            fired_on(&prompt, &["widget", "gizmo", "tester"]),
            vec!["widget"]
        );
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
    fn widget_fires_from_inside_a_channel_block() {
        let trig = vec!["widget".to_string()];
        let cands = [cand(
            "facts",
            "3gtR1J",
            "A sample fragment for the widget door.",
            "2026-01-01T00:00:00Z",
            &trig,
        )];
        let sel = select(CHANNEL, &cands, &HashSet::new(), &HashMap::new(), 2);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].trigger, "widget");
        assert_eq!(sel.fired[0].dig, "facts/kv-3gtR1J");
        assert_eq!(
            sel.fired[0].render(),
            "\u{1f6aa} widget \u{2192} A sample fragment for the widget door. (dig: facts/kv-3gtR1J)"
        );
    }

    #[test]
    fn already_fired_entry_is_skipped() {
        let trig = vec!["widget".to_string()];
        let cands = [cand("facts", "aaa", "v", "2026-01-01T00:00:00Z", &trig)];
        let fired: HashSet<(String, String)> = [("facts".to_string(), "aaa".to_string())]
            .into_iter()
            .collect();
        let sel = select("hi widget", &cands, &fired, &HashMap::new(), 2);
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
        let trig = vec!["widget".to_string()];
        let cands = [
            cand("facts", "same01", "a", "2026-01-01T00:00:00Z", &trig),
            cand("doors", "same01", "b", "2026-01-02T00:00:00Z", &trig),
        ];
        let fired: HashSet<(String, String)> = [("facts".to_string(), "same01".to_string())]
            .into_iter()
            .collect();
        let sel = select("widget", &cands, &fired, &HashMap::new(), 2);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].key, "doors");
    }

    #[test]
    fn budget_overflow_defers_the_most_fired() {
        let trig = vec!["widget".to_string()];
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
        let sel = select("widget", &cands, &HashSet::new(), &counts, 2);
        assert_eq!(sel.deferred, 1);
        let ids: Vec<&str> = sel.fired.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["ccc", "bbb"], "least-fired first");
    }

    #[test]
    fn tie_on_fire_count_breaks_to_oldest_ts() {
        let trig = vec!["widget".to_string()];
        let cands = [
            cand("d", "new", "n", "2026-06-01T00:00:00Z", &trig),
            cand("d", "old", "o", "2020-01-01T00:00:00Z", &trig),
        ];
        let sel = select("widget", &cands, &HashSet::new(), &HashMap::new(), 1);
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].id, "old");
    }

    #[test]
    fn no_match_is_an_empty_selection() {
        let trig = vec!["widget".to_string()];
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
