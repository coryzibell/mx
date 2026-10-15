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

mod classify;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub use classify::{Classified, NamesUs, Verdict, classify};

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
    /// Where the matching text came from: `typed`, `paste` or `channel`.
    pub origin: &'static str,
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
    let mut i = 0;
    while i < prompt.len() {
        let rest = &prompt[i..];
        if let Some(after) = rest.strip_prefix("</channel>") {
            out.push(' ');
            i += rest.len() - after.len();
            continue;
        }
        if is_channel_opener(rest) {
            if let Some(end) = channel_tag_end(prompt, i) {
                out.push(' ');
                i = end;
                continue;
            }
            // No closing bracket: this is not a real channel tag (a stray
            // "<channel" someone typed, or an unbalanced quote). Swallowing to
            // end-of-string would silently cost every door for the turn, so
            // treat the remainder as ordinary body text and stop stripping.
            out.push_str(rest);
            break;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

const CHANNEL_OPEN: &str = "<channel";

fn is_channel_opener(s: &str) -> bool {
    s.strip_prefix(CHANNEL_OPEN)
        .and_then(|r| r.chars().next())
        .is_some_and(|c| c.is_whitespace() || c == '>')
}

/// End (one past `>`) of the channel opener starting at `at`, scanning
/// quote-aware so an unbalanced `>` inside an attribute value does not end the
/// tag. `None` when no unquoted `>` follows.
fn channel_tag_end(s: &str, at: usize) -> Option<usize> {
    let mut in_quote = false;
    for (j, b) in s.bytes().enumerate().skip(at + CHANNEL_OPEN.len()) {
        match b {
            b'"' => in_quote = !in_quote,
            b'>' if !in_quote => return Some(j + 1),
            _ => {}
        }
    }
    None
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
/// Only bare tags count (`<name>`, `<name/>`, `</name>`; the harness never
/// writes attributes). A tag quoted in markdown is literal text: an opener or
/// self-closing tag right after a backtick, or a closer right before one, is
/// left alone and takes no part in pairing, so a report saying
/// `` `<task-notification>` `` does not open a block. A closer pairs with the
/// latest unclosed opener of its name, in linear time. An opener nothing pairs
/// with (no closer after it, or displaced by a later opener of the same name)
/// and a stray closer each lose only the tag. Each removed block or tag becomes
/// one space, so the text on either side stays separate tokens.
///
/// Known edges: (1) a body that quotes a tag WITHOUT backticks can pair wrongly
/// and leak part of that block's text, and nested blocks of the same name leak
/// the outer block's text; (2) when the next unbackticked tag after an
/// unbackticked opener with the same name is a closer, the text between them
/// is removed whoever wrote them (unless the opener already sits inside an
/// earlier removed block), including when that closer is quoted inside a
/// different block.
pub fn strip_harness_blocks(prompt: &str) -> String {
    let bytes = prompt.as_bytes();
    let tags: Vec<Tag<'_>> = prompt
        .match_indices('<')
        .filter_map(|(start, _)| {
            let (kind, name, len) = bare_tag(&prompt[start..])?;
            let end = start + len;
            let quoted = match kind {
                TagKind::Open | TagKind::SelfClosing => start > 0 && bytes[start - 1] == b'`',
                TagKind::Close => bytes.get(end) == Some(&b'`'),
            };
            (!quoted).then_some(Tag {
                start,
                end,
                kind,
                name,
            })
        })
        .collect();

    // block_end[i]: where the block opened by tag i ends. A closer pairs with
    // the latest unclosed opener of its name; an earlier one it displaced
    // stays unpaired, so an opener quoted inside a block of its own name closes
    // no later than that block's closer.
    let mut block_end: Vec<Option<usize>> = vec![None; tags.len()];
    let mut open: HashMap<&str, usize> = HashMap::new();
    for (i, tag) in tags.iter().enumerate() {
        match tag.kind {
            TagKind::Open => {
                open.insert(tag.name, i);
            }
            TagKind::Close => {
                if let Some(o) = open.remove(tag.name) {
                    block_end[o] = Some(tag.end);
                }
            }
            TagKind::SelfClosing => {}
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

/// Who wrote a stretch of the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Typed into the prompt, or anything not otherwise marked.
    Typed,
    /// A terminal paste Claude Code wrapped in a `<pasted_content>` block.
    Paste,
    /// The body of a `<channel>` block. Only `user_id` is kept: display names
    /// are spoofable.
    Channel { user_id: String },
}

impl Origin {
    pub fn label(&self) -> &'static str {
        match self {
            Origin::Typed => "typed",
            Origin::Paste => "paste",
            Origin::Channel { .. } => "channel",
        }
    }
}

/// One stretch of a prompt with a single origin. Phrases never match across
/// segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSegment {
    pub origin: Origin,
    pub text: String,
}

const PASTE_OPEN: &str = "<pasted_content id=\"";
const PASTE_CLOSE: &str = "</pasted_content id=\"";
const CHANNEL_CLOSE: &str = "</channel>";

/// The id and full length of the paste tag `prefix` starts `s` with:
/// `<pasted_content id="X">` or `</pasted_content id="X">`.
fn paste_tag<'a>(s: &'a str, prefix: &str) -> Option<(&'a str, usize)> {
    let rest = s.strip_prefix(prefix)?;
    let n = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        .count();
    (n > 0 && rest[n..].starts_with("\">")).then(|| (&rest[..n], prefix.len() + n + 2))
}

/// The value of attribute `name` in a channel opener's attribute text.
fn channel_attr(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let (value, tail) = match after.strip_prefix('"') {
            Some(q) => {
                let close = q.find('"')?;
                (&q[..close], &q[close + 1..])
            }
            None => {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        // A key with spaces before it was a bare word; only its last token is
        // the attribute name.
        if key.rsplit(char::is_whitespace).next() == Some(name) {
            return Some(value.to_string());
        }
        rest = tail;
    }
}

fn push_typed(out: &mut Vec<RawSegment>, text: &str) {
    let text = strip_channel_tags(text);
    if !text.trim().is_empty() {
        out.push(RawSegment {
            origin: Origin::Typed,
            text,
        });
    }
}

/// Cut a prompt (already through `strip_harness_blocks`) into typed text,
/// terminal pastes and channel bodies.
///
/// Scanning left to right, whichever block opener comes first defines a block,
/// and its body is opaque to the other kind. A paste opens at
/// `<pasted_content id="X">` and closes at the first later
/// `</pasted_content id="X">` with the same X; the newline just inside each tag
/// belongs to the envelope. Channel tags inside a paste body are stripped and
/// its text stays in the paste. A channel block opens at `<channel …>`, keeps
/// only `user_id`, and runs to the next `</channel>`; paste tags inside it are
/// plain body text. Attribute text never becomes match text, and
/// whitespace-only typed fragments are dropped.
///
/// Unterminated openers never swallow the turn: a paste opener nothing closes
/// is ordinary text, and a channel opener with no `>` before the next paste is
/// text, as is every channel opener after it until that paste.
///
/// Known edges:
/// - A channel opener with `>` but no `</channel>` labels the rest of the text
///   `channel`. Routing is `{house}` either way; only the origin in telemetry
///   differs.
/// - A paste body that contains its own session's closer (same id) closes
///   early there, and the remainder becomes typed text (admitted).
pub fn segment(prompt: &str) -> Vec<RawSegment> {
    let mut closers: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for (i, _) in prompt.match_indices(PASTE_CLOSE) {
        if let Some((id, len)) = paste_tag(&prompt[i..], PASTE_CLOSE) {
            closers.entry(id).or_default().push((i, i + len));
        }
    }
    // (body start, closer start, closer end) of a paste opening at `i`.
    let paste_at = |i: usize| {
        let (id, len) = paste_tag(&prompt[i..], PASTE_OPEN)?;
        let body_start = i + len;
        let v = closers.get(id)?;
        let &(close_start, close_end) = v.get(v.partition_point(|&(s, _)| s < body_start))?;
        Some((body_start, close_start, close_end))
    };

    let mut out = Vec::new();
    let mut pos = 0;
    let mut at = 0;
    let mut channels = true;
    while let Some(off) = prompt[at..].find('<') {
        let i = at + off;
        at = i + 1;
        if let Some((body_start, close_start, close_end)) = paste_at(i) {
            push_typed(&mut out, &prompt[pos..i]);
            let body = &prompt[body_start..close_start];
            let body = body.strip_prefix('\n').unwrap_or(body);
            let body = body.strip_suffix('\n').unwrap_or(body);
            out.push(RawSegment {
                origin: Origin::Paste,
                text: strip_channel_tags(body),
            });
            pos = close_end;
            at = close_end;
            channels = true;
        } else if channels && is_channel_opener(&prompt[i..]) {
            // An opener with no `>` of its own must not borrow one from a
            // later paste tag and swallow that paste into its attributes.
            let next_paste = prompt[i..]
                .match_indices(PASTE_OPEN)
                .map(|(o, _)| i + o)
                .find(|&j| paste_at(j).is_some())
                .unwrap_or(prompt.len());
            let Some(tag_end) = channel_tag_end(&prompt[..next_paste], i) else {
                channels = false;
                continue;
            };
            push_typed(&mut out, &prompt[pos..i]);
            let user_id = channel_attr(&prompt[i + CHANNEL_OPEN.len()..tag_end - 1], "user_id")
                .unwrap_or_default();
            let (body_end, next) = match prompt[tag_end..].find(CHANNEL_CLOSE) {
                Some(o) => (tag_end + o, tag_end + o + CHANNEL_CLOSE.len()),
                None => (prompt.len(), prompt.len()),
            };
            out.push(RawSegment {
                origin: Origin::Channel { user_id },
                text: strip_channel_tags(&prompt[tag_end..body_end]),
            });
            pos = next;
            at = next;
        }
    }
    push_typed(&mut out, &prompt[pos..]);
    out
}

/// A segment after routing has placed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub origin: Origin,
    pub text: String,
    /// The classifier's judgement. Only pastes are judged.
    pub classified: Option<Classified>,
    /// Places `∅`: no door matches in it. Otherwise places `{house}`.
    pub quarantined: bool,
}

/// Segment, classify and route a raw prompt.
///
/// House mode only, and only tagged pastes are judged: a paste that reads as
/// someone else's prose is quarantined. Typed text and channel bodies are never
/// classified and always place `{house}`, as do Ours, Unsure and gated pastes.
pub fn analyze(prompt: &str, names: &NamesUs) -> Vec<Segment> {
    segment(&strip_harness_blocks(prompt))
        .into_iter()
        .map(|s| {
            // Shelved: an untagged fallback that judges long typed and channel
            // text would go here. It held the user's own long typed messages.
            let classified = (s.origin == Origin::Paste).then(|| classify(&s.text, names));
            // P2b: a book mode routes this to `book:<slug>` instead of `∅`.
            let quarantined = classified
                .as_ref()
                .is_some_and(|c| c.verdict == Verdict::ForeignProse);
            Segment {
                origin: s.origin,
                text: s.text,
                classified,
                quarantined,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DoorStatus {
    Fired,
    /// Matched, lost the budget. Not recorded, so it stays eligible.
    Deferred,
    /// Matched only in quarantined segments. Never fired, never recorded.
    Held,
}

impl DoorStatus {
    pub fn label(self) -> &'static str {
        match self {
            DoorStatus::Fired => "FIRED",
            DoorStatus::Deferred => "DEFERRED",
            DoorStatus::Held => "HELD",
        }
    }
}

/// One door that matched somewhere in the prompt, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoorMatch {
    pub door: FiredDoor,
    pub status: DoorStatus,
    /// 1-based index into the segments.
    pub segment: usize,
}

/// Everything one prompt produced, for `check --explain`.
#[derive(Debug, Clone, PartialEq)]
pub struct Explanation {
    pub segments: Vec<Segment>,
    pub doors: Vec<DoorMatch>,
}

impl Explanation {
    pub fn selection(&self) -> Selection {
        let count = |s| self.doors.iter().filter(|d| d.status == s).count();
        Selection {
            fired: self
                .doors
                .iter()
                .filter(|d| d.status == DoorStatus::Fired)
                .map(|d| d.door.clone())
                .collect(),
            deferred: count(DoorStatus::Deferred),
        }
    }
}

/// The most specific trigger a candidate matches across `segs`, and the index
/// of the segment it matched in. Ties go to the stored order within a
/// segment, then to the earlier segment.
fn best_hit<'s>(
    cand: &TriggeredRef<'_>,
    segs: impl Iterator<Item = (usize, &'s [String])>,
) -> Option<(String, usize)> {
    let mut best: Option<(String, usize)> = None;
    for (n, toks) in segs {
        let hits = triggers::match_triggers(toks, cand.triggers, false);
        // Report the MOST SPECIFIC trigger, not whichever was stored first.
        // An entry carrying both `gerf` and `gerf_slips` should say
        // `gerf_slips` fired on "gerf_slips is permanent" — the longer match
        // covers more of the message and is what the reader needs to see.
        let Some(t) = hits.into_iter().reduce(|best, t| {
            if specificity(&t) > specificity(&best) {
                t
            } else {
                best
            }
        }) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(b, _)| specificity(&t) > specificity(b))
        {
            best = Some((t, n));
        }
    }
    best
}

/// Decide which doors open for one prompt, and say why, held doors included.
///
/// `already_fired` holds the `(key, entry id)` pairs this session has seen, and
/// `fire_counts` the all-time count per pair. Ordering is fewest all-time fires
/// first, then oldest `ts` — a door nobody has opened yet outranks a familiar
/// one, so the budget goes to what the reader is least likely to already hold.
///
/// Only human text is matched: harness blocks are removed whole, then the
/// prompt is segmented and each segment is matched on its own. A door matching
/// in several admitted segments fires once; one matching only in quarantined
/// segments is held.
pub fn explain(
    prompt: &str,
    names: &NamesUs,
    candidates: &[TriggeredRef<'_>],
    already_fired: &HashSet<(String, String)>,
    fire_counts: &HashMap<(String, String), u64>,
    budget: usize,
) -> Explanation {
    run(
        prompt,
        names,
        candidates,
        already_fired,
        fire_counts,
        budget,
        true,
    )
}

/// `explain`, reduced to what fires and how many deferred. Held doors are
/// never computed: the hook never prints them, and quarantined segments are
/// not even tokenized.
pub fn select(
    prompt: &str,
    names: &NamesUs,
    candidates: &[TriggeredRef<'_>],
    already_fired: &HashSet<(String, String)>,
    fire_counts: &HashMap<(String, String), u64>,
    budget: usize,
) -> Selection {
    run(
        prompt,
        names,
        candidates,
        already_fired,
        fire_counts,
        budget,
        false,
    )
    .selection()
}

fn run(
    prompt: &str,
    names: &NamesUs,
    candidates: &[TriggeredRef<'_>],
    already_fired: &HashSet<(String, String)>,
    fire_counts: &HashMap<(String, String), u64>,
    budget: usize,
    with_held: bool,
) -> Explanation {
    let segments = analyze(prompt, names);
    // Stemming OFF: doors are proper nouns, and the English stemmer folds
    // Tagalog "ayos" onto "ayo". Morphology is the enemy here.
    let tokens: Vec<Vec<String>> = segments
        .iter()
        .map(|s| {
            if with_held || !s.quarantined {
                triggers::tokens(&s.text, false)
            } else {
                Vec::new()
            }
        })
        .collect();
    let pool = |quarantined: bool| {
        segments
            .iter()
            .zip(&tokens)
            .enumerate()
            .filter(move |(_, (s, t))| s.quarantined == quarantined && !t.is_empty())
            .map(|(n, (_, t))| (n, t.as_slice()))
    };

    let mut matched: Vec<(u64, &str, DoorMatch)> = Vec::new();
    let mut held: Vec<(u64, &str, DoorMatch)> = Vec::new();
    for cand in candidates {
        let pair = (cand.key.to_string(), cand.id.to_string());
        if already_fired.contains(&pair) {
            continue;
        }
        let (hit, status) = match best_hit(cand, pool(false)) {
            Some(h) => (h, DoorStatus::Fired),
            None if with_held => match best_hit(cand, pool(true)) {
                Some(h) => (h, DoorStatus::Held),
                None => continue,
            },
            None => continue,
        };
        let (trigger, n) = hit;
        let count = fire_counts.get(&pair).copied().unwrap_or(0);
        let entry = (
            count,
            cand.ts,
            DoorMatch {
                door: FiredDoor {
                    key: pair.0,
                    id: pair.1,
                    trigger,
                    fragment: derive_fragment(cand.value, cand.fragment),
                    dig: dig_pointer(cand.key, cand.id, cand.memory),
                    origin: segments[n].origin.label(),
                },
                status,
                segment: n + 1,
            },
        );
        match status {
            DoorStatus::Held => held.push(entry),
            _ => matched.push(entry),
        }
    }

    let order = |a: &(u64, &str, DoorMatch), b: &(u64, &str, DoorMatch)| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.door.key.cmp(&b.2.door.key))
            .then_with(|| a.2.door.id.cmp(&b.2.door.id))
    };
    matched.sort_by(order);
    held.sort_by(order);
    let doors = matched
        .into_iter()
        .enumerate()
        .map(|(i, (_, _, mut m))| {
            if i >= budget {
                m.status = DoorStatus::Deferred;
            }
            m
        })
        .chain(held.into_iter().map(|h| h.2))
        .collect();
    Explanation { segments, doors }
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
        let sel = select(
            CHANNEL,
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
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
            &NamesUs::default(),
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
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
        assert_eq!(sel.fired[0].trigger, "gerf_slips");

        // And a message with only the short form still reports the short form.
        let sel = select(
            "ask gerf",
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
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
            &NamesUs::default(),
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
        let prompt = "<system-reminderz>keep</system-reminderz> <System-Reminder>keep2</System-Reminder> <local-command->keep3</local-command-> <local-command-Stdout>keep4</local-command-Stdout>";
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
    fn report_quoting_its_own_opener_in_backticks_is_removed_whole() {
        let prompt = concat!(
            "<task-notification>\n<status>completed</status>\n",
            "<result>Ran kv update on keurig, then stripped `<task-notification>` blocks.</result>\n",
            "</task-notification>\nhi"
        );
        assert_eq!(strip_harness_blocks(prompt), " \nhi");
        assert!(fired_on(prompt, &["keurig"]).is_empty());
        assert_eq!(fired_on(prompt, &["keurig", "hi"]), vec!["hi"]);
    }

    #[test]
    fn unpaired_opener_loses_only_its_tag_between_blocks() {
        let prompt = "<system-reminder>a <system-reminder>b</system-reminder> widget <system-reminder>c</system-reminder>";
        assert_eq!(strip_harness_blocks(prompt), " a   widget  ");
    }

    #[test]
    fn backtick_quoted_closer_inside_a_report_is_literal() {
        let prompt =
            "<task-notification>the `</task-notification>` closer, sprocket</task-notification>";
        assert_eq!(strip_harness_blocks(prompt), " ");
        assert!(fired_on(prompt, &["sprocket"]).is_empty());
    }

    #[test]
    fn known_edge_unbackticked_quoted_closer_ends_the_block_early() {
        // Pinned: a body quoting its own closer without backticks ends there,
        // so the tail leaks. It leaks that block's text only.
        let prompt =
            "<task-notification>the </task-notification> closer, sprocket</task-notification>";
        assert_eq!(strip_harness_blocks(prompt), "  closer, sprocket ");
        assert_eq!(fired_on(prompt, &["sprocket"]), vec!["sprocket"]);
    }

    #[test]
    fn known_edge_human_bare_opener_and_later_bare_closer_remove_the_text_between() {
        // Pinned: an unbackticked opener and a later unbackticked closer of the
        // same name, whoever wrote them. Pairing cannot tell this from a
        // harness block, so the text between goes.
        let prompt = "my <system-reminder> said keurig, and that </system-reminder> was odd widget";
        assert_eq!(strip_harness_blocks(prompt), "my   was odd widget");
        assert_eq!(fired_on(prompt, &["keurig", "widget"]), vec!["widget"]);
    }

    #[test]
    fn known_edge_closer_quoted_in_another_block_pairs_with_a_human_opener() {
        // Pinned: cross-name. The human's bare opener pairs with a closer a
        // system-reminder body quotes, so the text between goes.
        let prompt = "I typed <task-notification> keurig <system-reminder>quote </task-notification> x</system-reminder>";
        assert_eq!(strip_harness_blocks(prompt), "I typed   x ");
        assert!(fired_on(prompt, &["keurig"]).is_empty());
    }

    #[test]
    fn unbackticked_quotes_in_two_reports_keep_the_human_between() {
        let prompt = "<task-notification>r1 stripped <task-notification> blocks</task-notification>\nHUMAN keurig\n<task-notification>r2 the </task-notification> closer</task-notification>";
        assert_eq!(
            strip_harness_blocks(prompt),
            " r1 stripped  \nHUMAN keurig\n  closer "
        );
        assert_eq!(fired_on(prompt, &["keurig"]), vec!["keurig"]);
    }

    #[test]
    fn bare_mention_before_a_report_quoting_a_closer_keeps_the_human_text() {
        let prompt = "I pasted <task-notification> earlier, keurig please <task-notification>report mentions </task-notification> lol</task-notification>";
        assert_eq!(
            strip_harness_blocks(prompt),
            "I pasted   earlier, keurig please   lol "
        );
        assert_eq!(fired_on(prompt, &["keurig"]), vec!["keurig"]);
    }

    #[test]
    fn human_bare_closer_after_an_unbackticked_report_keeps_the_human_text() {
        let prompt = "<task-notification>stripped <task-notification> blocks</task-notification>\nkeurig: the </task-notification> closer is weird";
        assert_eq!(fired_on(prompt, &["keurig"]), vec!["keurig"]);
    }

    #[test]
    fn backticks_inside_a_real_body_do_not_quote_its_tags() {
        let prompt = "<local-command-stdout>`widget`</local-command-stdout> gizmo";
        assert_eq!(strip_harness_blocks(prompt), "  gizmo");
        assert_eq!(fired_on(prompt, &["widget", "gizmo"]), vec!["gizmo"]);
    }

    #[test]
    fn backtick_adjacent_opener_is_literal_and_does_not_pair() {
        let prompt = "`<system-reminder>` widget</system-reminder> gizmo";
        assert_eq!(
            strip_harness_blocks(prompt),
            "`<system-reminder>` widget  gizmo"
        );
        assert_eq!(
            fired_on(prompt, &["widget", "gizmo"]),
            vec!["gizmo", "widget"]
        );
    }

    #[test]
    fn backtick_adjacent_closer_is_literal_and_does_not_pair() {
        let prompt = "<system-reminder>note `</system-reminder>` widget";
        assert_eq!(
            strip_harness_blocks(prompt),
            " note `</system-reminder>` widget"
        );
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn backtick_adjacent_self_closing_is_literal() {
        let prompt = "`<system-reminder/>` widget `<local-command-caveat/>`";
        assert_eq!(strip_harness_blocks(prompt), prompt);
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn human_bare_opener_before_a_real_block_keeps_the_human_text() {
        // Poppy P4.
        let prompt = "why does <system-reminder> keep firing keurig\n<system-reminder>harness note</system-reminder>";
        assert_eq!(
            strip_harness_blocks(prompt),
            "why does   keep firing keurig\n "
        );
        assert_eq!(fired_on(prompt, &["keurig", "harness"]), vec!["keurig"]);
    }

    #[test]
    fn quoted_opener_and_quoted_closer_in_two_reports_keep_the_human_between() {
        // Poppy P5.
        let prompt = concat!(
            "<task-notification>r1 stripped `<task-notification>` blocks</task-notification>",
            "\nHUMAN keurig\n",
            "<task-notification>r2 the `</task-notification>` closer</task-notification>"
        );
        assert_eq!(strip_harness_blocks(prompt), " \nHUMAN keurig\n ");
        assert_eq!(
            fired_on(prompt, &["keurig", "stripped", "closer"]),
            vec!["keurig"]
        );
    }

    #[test]
    fn human_bare_closer_after_a_report_loses_only_the_tag() {
        // Poppy P6.
        let prompt = "<task-notification>stripped `<task-notification>` blocks</task-notification>\nkeurig: the </task-notification> closer is weird";
        assert_eq!(
            strip_harness_blocks(prompt),
            " \nkeurig: the   closer is weird"
        );
        assert_eq!(fired_on(prompt, &["keurig", "stripped"]), vec!["keurig"]);
    }

    #[test]
    fn backticked_mention_before_a_real_block_keeps_the_human_text() {
        // Diffi F6.
        let prompt = "why does `<system-reminder>` keep firing keurig? <system-reminder>harness note</system-reminder>";
        let out = strip_harness_blocks(prompt);
        assert!(out.contains("keep firing keurig"), "{out:?}");
        assert!(!out.contains("harness note"), "{out:?}");
        assert_eq!(fired_on(prompt, &["keurig", "harness"]), vec!["keurig"]);
    }

    #[test]
    fn bare_mention_before_a_real_block_keeps_the_human_text() {
        // Diffi F6.
        let prompt = "I pasted <task-notification> earlier, keurig please <task-notification>report</task-notification>";
        let out = strip_harness_blocks(prompt);
        assert!(out.contains("keurig please"), "{out:?}");
        assert!(!out.contains("report"), "{out:?}");
        assert_eq!(fired_on(prompt, &["keurig", "report"]), vec!["keurig"]);
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
    fn known_edge_nested_same_name_leaks_the_outer_text() {
        // Pinned: the inner opener displaces the outer one, so only the inner
        // block goes and the outer block's own text leaks. Agent text only.
        let prompt =
            "<system-reminder>a<system-reminder>b</system-reminder>c</system-reminder> widget";
        assert_eq!(strip_harness_blocks(prompt), " a c  widget");
        assert!(fired_on(prompt, &["b"]).is_empty());
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
        const PIECES: [&str; 26] = [
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
            "<pasted_content id=\"a\">",
            "</pasted_content id=\"a\">",
            "</pasted_content id=\"b\">",
            "<pasted_content id=\"",
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
            let _ = segment(&s);
            let _ = fired_on(&s, &["x", "system-reminder"]);
        }
    }

    #[test]
    fn many_unbalanced_openers_stay_linear() {
        use std::time::{Duration, Instant};
        for unit in ["<system-reminder>x ", "<local-command-stdout>x "] {
            let prompt = format!("{} widget", unit.repeat(60_000));
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
    fn many_unclosed_paste_openers_stay_linear() {
        use std::time::{Duration, Instant};
        let prompt = format!(
            "{} widget {}",
            "<pasted_content id=\"aaaa\">x ".repeat(60_000),
            "</pasted_content id=\"bbbb\"> ".repeat(60_000)
        );
        let t = Instant::now();
        let segs = segment(&prompt);
        let took = t.elapsed();
        assert_eq!(segs.len(), 1);
        assert!(took < Duration::from_millis(500), "took {took:?}");
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

    #[test]
    fn harness_blocks_are_stripped_before_channel_tags() {
        let prompt = concat!(
            "<system-reminder>quoted <channel oops</system-reminder>",
            r#"<channel source="matrix" user="tester">widget</channel>"#
        );
        assert_eq!(
            fired_on(prompt, &["widget", "quoted", "tester"]),
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
        let sel = select(
            CHANNEL,
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
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
        let sel = select(
            "hi widget",
            &NamesUs::default(),
            &cands,
            &fired,
            &HashMap::new(),
            2,
        );
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
        let sel = select(
            "widget",
            &NamesUs::default(),
            &cands,
            &fired,
            &HashMap::new(),
            2,
        );
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
        let sel = select(
            "widget",
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &counts,
            2,
        );
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
        let sel = select(
            "widget",
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            1,
        );
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].id, "old");
    }

    #[test]
    fn no_match_is_an_empty_selection() {
        let trig = vec!["widget".to_string()];
        let cands = [cand("d", "aaa", "a", "2026-01-01T00:00:00Z", &trig)];
        let sel = select(
            "hello there",
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
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
            select(
                "ayos lang",
                &NamesUs::default(),
                &cands,
                &HashSet::new(),
                &HashMap::new(),
                2
            )
            .fired
            .is_empty()
        );
        assert_eq!(
            select(
                "switched to ayo-mirage",
                &NamesUs::default(),
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

    // ---- segment ----

    const WALDEN: &str = include_str!("../tests/fixtures/doors/pd/walden.txt");
    const OPENER: &str = include_str!("../tests/fixtures/doors/syn/syn-chapter-opener.txt");
    const CV: &str = include_str!("../tests/fixtures/doors/syn/syn-cv.txt");

    fn seg(origin: Origin, text: &str) -> RawSegment {
        RawSegment {
            origin,
            text: text.to_string(),
        }
    }

    fn channel(user_id: &str) -> Origin {
        Origin::Channel {
            user_id: user_id.to_string(),
        }
    }

    /// A paste in Claude Code's envelope.
    fn paste(id: &str, body: &str) -> String {
        format!("<pasted_content id=\"{id}\">\n{body}\n</pasted_content id=\"{id}\">")
    }

    #[test]
    fn paste_envelope_is_cut_exactly() {
        // The Step 0 shape: typed, blank line, tagged paste, blank line, typed.
        let prompt = format!(
            "please read this: \n\n{}\n\n and say ok",
            paste("f23b", "line one\n\nline two\n")
        );
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "please read this: \n\n"),
                seg(Origin::Paste, "line one\n\nline two\n"),
                seg(Origin::Typed, "\n\n and say ok"),
            ]
        );
    }

    #[test]
    fn only_a_closer_with_the_same_id_closes_a_paste() {
        let prompt = "<pasted_content id=\"aaaa\">\nx\n</pasted_content id=\"bbbb\">\ny\n</pasted_content id=\"aaaa\">\n</pasted_content id=\"aaaa\">";
        assert_eq!(
            segment(prompt),
            vec![
                seg(Origin::Paste, "x\n</pasted_content id=\"bbbb\">\ny"),
                seg(Origin::Typed, "\n</pasted_content id=\"aaaa\">"),
            ],
            "the first matching closer ends it"
        );
    }

    #[test]
    fn unclosed_paste_opener_is_ordinary_text() {
        let prompt = "hi <pasted_content id=\"aaaa\">\nwidget";
        assert_eq!(segment(prompt), vec![seg(Origin::Typed, prompt)]);
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);

        // And it does not stop a later real paste from being cut out.
        let prompt = format!(
            "<pasted_content id=\"zz\">oops {} tail",
            paste("aa", "body")
        );
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "<pasted_content id=\"zz\">oops "),
                seg(Origin::Paste, "body"),
                seg(Origin::Typed, " tail"),
            ]
        );
    }

    #[test]
    fn malformed_paste_tags_are_text() {
        for prompt in [
            "<pasted_content id=\"\">\nx\n</pasted_content id=\"\">",
            "<pasted_content id=\"a b\">\nx\n</pasted_content id=\"a b\">",
            "<pasted_content id=aaaa>\nx\n</pasted_content id=aaaa>",
        ] {
            assert_eq!(
                segment(prompt),
                vec![seg(Origin::Typed, prompt)],
                "{prompt:?}"
            );
        }
    }

    #[test]
    fn a_pasted_transcript_stays_one_paste_and_loses_channel_attributes() {
        let body = format!("before\n{CHANNEL}\nafter");
        let segs = segment(&paste("aaaa", &body));
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!(segs[0].origin, Origin::Paste);
        assert!(segs[0].text.contains("good morning widget"));
        assert!(!segs[0].text.contains("tester"), "{:?}", segs[0].text);
        assert!(!segs[0].text.contains("delta"));
    }

    #[test]
    fn paste_tags_inside_a_channel_body_are_plain_body_text() {
        let prompt = "<channel source=\"matrix\" user_id=\"@m:e\">\nhi\n<pasted_content id=\"ab12\">\nx\n</pasted_content id=\"ab12\">\nzephyrine\n</channel>";
        let segs = segment(prompt);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!(segs[0].origin, channel("@m:e"));
        assert!(segs[0].text.contains("zephyrine"));
        assert!(segs[0].text.contains("<pasted_content id=\"ab12\">"));
    }

    #[test]
    fn the_first_opener_wins() {
        // Channel first: its body runs to `</channel>`, even through a paste
        // whose closer lies beyond it.
        let prompt = "<channel user_id=\"u\">a <pasted_content id=\"x\">\nb\n</channel> c\n</pasted_content id=\"x\"> d";
        assert_eq!(
            segment(prompt),
            vec![
                seg(channel("u"), "a <pasted_content id=\"x\">\nb\n"),
                seg(Origin::Typed, " c\n</pasted_content id=\"x\"> d"),
            ]
        );
        // Paste first: a channel closer inside it is stripped, and a later one
        // in typed text goes too.
        let prompt = format!(
            "{} tail</channel>",
            paste("x", "a <channel user_id=\"u\">b")
        );
        assert_eq!(
            segment(&prompt),
            vec![seg(Origin::Paste, "a  b"), seg(Origin::Typed, " tail ")]
        );
    }

    #[test]
    fn known_edge_channel_without_a_closer_runs_to_the_end() {
        let prompt = format!("hi <channel user_id=\"u\">body {}", paste("x", "p"));
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "hi "),
                seg(
                    channel("u"),
                    "body <pasted_content id=\"x\">\np\n</pasted_content id=\"x\">"
                ),
            ]
        );
    }

    #[test]
    fn known_edge_a_paste_quoting_its_own_closer_ends_early() {
        let body = "a\n</pasted_content id=\"x\">\nb";
        assert_eq!(
            segment(&paste("x", body)),
            vec![
                seg(Origin::Paste, "a"),
                seg(Origin::Typed, "\nb\n</pasted_content id=\"x\">"),
            ]
        );
    }

    #[test]
    fn unterminated_channel_opener_does_not_stop_a_later_paste() {
        // One quote in the body makes the quote count even, so an unbounded
        // scan would find the paste closer's `>` outside any quote.
        let prompt = format!("<channel user_id=\"u {} tail", paste("x", "say \"p"));
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "<channel user_id=\"u "),
                seg(Origin::Paste, "say \"p"),
                seg(Origin::Typed, " tail"),
            ]
        );
    }

    #[test]
    fn a_bare_typed_channel_word_does_not_swallow_a_later_paste() {
        let prompt = format!("what does <channel do here?\n\n{}", paste("ab12", WALDEN));
        let segs = analyze(&prompt, &NamesUs::default());
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[0].origin, Origin::Typed);
        assert_eq!(segs[0].text, "what does <channel do here?\n\n");
        assert_eq!(segs[1].origin, Origin::Paste);
        assert!(segs[1].quarantined, "a foreign paste is held");
    }

    #[test]
    fn an_unterminated_channel_attribute_does_not_swallow_a_later_paste() {
        let body = "she said \"go home and never come back to the old mill";
        let prompt = format!(
            "see <channel user_id=\"u \n\n{}\nthanks",
            paste("ab12", body)
        );
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "see <channel user_id=\"u \n\n"),
                seg(Origin::Paste, body),
                seg(Origin::Typed, "\nthanks"),
            ]
        );
    }

    #[test]
    fn a_channel_after_a_paste_is_a_channel_again() {
        let prompt = format!(
            "<channel oops {}\n<channel user_id=\"v\">hi</channel>",
            paste("ab12", "p")
        );
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "<channel oops "),
                seg(Origin::Paste, "p"),
                seg(channel("v"), "hi"),
            ]
        );
    }

    #[test]
    fn channel_body_is_its_own_segment_keyed_by_user_id() {
        let prompt = format!("hi {CHANNEL} bye");
        assert_eq!(
            segment(&prompt),
            vec![
                seg(Origin::Typed, "hi "),
                seg(channel("@j:s"), "\ngood morning widget\n"),
                seg(Origin::Typed, " bye"),
            ]
        );
        // A display name is not an identity; only user_id is read.
        let segs = segment(r#"<channel user="@spoof:s" xuser_id="@x:s">hi</channel>"#);
        assert_eq!(segs, vec![seg(channel(""), "hi")]);
        let segs = segment(r#"<channel room_name="a > b" user_id="@j:s">hi</channel>"#);
        assert_eq!(segs, vec![seg(channel("@j:s"), "hi")]);
    }

    #[test]
    fn channel_opener_without_a_bracket_is_text() {
        let prompt = "<channel user_id=\"unclosed widget";
        assert_eq!(segment(prompt), vec![seg(Origin::Typed, prompt)]);
        assert_eq!(fired_on(prompt, &["widget"]), vec!["widget"]);
    }

    #[test]
    fn whitespace_only_typed_fragments_are_dropped() {
        let prompt = "<channel user_id=\"u\">x</channel>\n\n<channel user_id=\"v\">y</channel>\n";
        assert_eq!(
            segment(prompt),
            vec![seg(channel("u"), "x"), seg(channel("v"), "y")]
        );
        assert!(segment("  \n ").is_empty());
    }

    #[test]
    fn phrases_never_match_across_a_segment_boundary() {
        let prompt = format!("fair\n\n{}", paste("aaaa", "haven"));
        assert!(fired_on(&prompt, &["fair haven"]).is_empty());
        assert_eq!(fired_on("fair haven", &["fair haven"]), vec!["fair haven"]);
    }

    /// What `main` fired before segmentation: one token stream for the whole
    /// prompt, harness blocks and channel tags stripped.
    fn legacy_fired(prompt: &str, trigs: &[&str]) -> Vec<String> {
        let toks = triggers::tokens(&strip_channel_tags(&strip_harness_blocks(prompt)), false);
        let mut out: Vec<String> = trigs
            .iter()
            .filter(|t| !triggers::match_triggers(&toks, &[t.to_string()], false).is_empty())
            .map(|t| t.to_string())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn untagged_short_prompts_fire_exactly_as_before() {
        for (prompt, trigs) in [
            (
                "<task-notification>agent quoted widget and gizmo</task-notification><system-reminder>gizmo widget sprocket</system-reminder>\nwhat about the widget?",
                &["widget", "gizmo", "sprocket"][..],
            ),
            ("hi <system-reminder oops widget", &["widget"]),
            (
                "the <system-reminder thing keeps opening keurig -> please fix",
                &["keurig"],
            ),
            ("the widget</task-notification>works", &["widget", "works"]),
            (
                "the<system-reminder/>widget <local-command-caveat/>works",
                &["widget"],
            ),
            (
                "my <system-reminder> said keurig, and that </system-reminder> was odd widget",
                &["keurig", "widget"],
            ),
            ("gerf_slips is permanent", &["gerf", "gerf_slips"]),
            ("switched to ayo-mirage, ayos lang", &["ayo-", "ayos"]),
        ] {
            assert!(
                analyze(prompt, &NamesUs::default())
                    .iter()
                    .all(|s| s.origin == Origin::Typed && s.classified.is_none()),
                "{prompt:?}"
            );
            assert_eq!(
                fired_on(prompt, trigs),
                legacy_fired(prompt, trigs),
                "{prompt:?}"
            );
            assert!(!legacy_fired(prompt, trigs).is_empty(), "{prompt:?}");
        }
    }

    // ---- routing ----

    fn explain_on(prompt: &str, trigs: &[&str], budget: usize) -> Explanation {
        let owned: Vec<Vec<String>> = trigs.iter().map(|t| vec![t.to_string()]).collect();
        let cands: Vec<TriggeredRef<'_>> = owned
            .iter()
            .enumerate()
            .map(|(n, t)| TriggeredRef {
                key: "d",
                id: trigs[n],
                value: "v",
                ts: "2026-01-01T00:00:00Z",
                triggers: t,
                fragment: None,
                memory: None,
            })
            .collect();
        explain(
            prompt,
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            budget,
        )
    }

    fn outcome(ex: &Explanation) -> Vec<(String, DoorStatus, &'static str, usize)> {
        ex.doors
            .iter()
            .map(|d| (d.door.trigger.clone(), d.status, d.door.origin, d.segment))
            .collect()
    }

    #[test]
    fn foreign_paste_is_quarantined_and_typed_text_still_fires() {
        let prompt = format!(
            "about huckleberries? \n\n{}\n\n keep it short",
            paste("aaaa", WALDEN)
        );
        let ex = explain_on(&prompt, &["huckleberries", "fair haven"], 2);
        assert_eq!(
            ex.segments[1].classified.as_ref().map(|c| c.verdict),
            Some(Verdict::ForeignProse)
        );
        assert!(ex.segments[1].quarantined);
        assert_eq!(
            outcome(&ex),
            vec![
                ("huckleberries".into(), DoorStatus::Fired, "typed", 1),
                ("fair haven".into(), DoorStatus::Held, "paste", 2),
            ]
        );
        let sel = ex.selection();
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].origin, "typed");
        assert_eq!(sel.deferred, 0, "a held door is not deferred");
    }

    #[test]
    fn held_doors_do_not_spend_the_budget() {
        let prompt = format!("widget\n\n{}", paste("aaaa", WALDEN));
        let ex = explain_on(&prompt, &["fair haven", "huckleberries", "widget"], 1);
        let sel = ex.selection();
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(sel.fired[0].trigger, "widget");
        assert_eq!(sel.deferred, 0);
    }

    #[test]
    fn short_and_ours_pastes_keep_todays_behaviour() {
        let prompt = paste("aaaa", "a short note about fair haven");
        assert_eq!(
            outcome(&explain_on(&prompt, &["fair haven"], 2)),
            vec![("fair haven".into(), DoorStatus::Fired, "paste", 1)]
        );
        let ex = explain_on(&paste("aaaa", CV), &["eurocodes"], 2);
        assert_eq!(
            ex.segments[0].classified.as_ref().map(|c| c.verdict),
            Some(Verdict::Ours)
        );
        assert_eq!(
            outcome(&ex),
            vec![("eurocodes".into(), DoorStatus::Fired, "paste", 1)]
        );
    }

    #[test]
    fn only_a_tagged_paste_is_ever_held() {
        assert!(WALDEN.chars().count() >= 600);
        let typed = explain_on(WALDEN, &["fair haven"], 2);
        assert_eq!(typed.segments.len(), 1);
        assert!(typed.segments[0].classified.is_none());
        assert_eq!(
            outcome(&typed),
            vec![("fair haven".into(), DoorStatus::Fired, "typed", 1)]
        );

        let body = format!("<channel source=\"matrix\" user_id=\"@j:s\">\n{WALDEN}\n</channel>");
        let ex = explain_on(&body, &["fair haven"], 2);
        assert_eq!(ex.segments[0].origin, channel("@j:s"));
        assert!(ex.segments[0].classified.is_none());
        assert_eq!(
            outcome(&ex),
            vec![("fair haven".into(), DoorStatus::Fired, "channel", 1)]
        );

        let ex = explain_on(&paste("aaaa", WALDEN), &["fair haven"], 2);
        assert_eq!(
            outcome(&ex),
            vec![("fair haven".into(), DoorStatus::Held, "paste", 1)]
        );
    }

    #[test]
    fn select_never_computes_held_doors() {
        let prompt = format!("widget\n\n{}", paste("aaaa", WALDEN));
        let trig = vec!["fair haven".to_string()];
        let wid = vec!["widget".to_string()];
        let cands = [
            cand("d", "aaa", "v", "2026-01-01T00:00:00Z", &trig),
            cand("d", "bbb", "v", "2026-01-01T00:00:00Z", &wid),
        ];
        let none = HashSet::new();
        let counts = HashMap::new();
        let names = NamesUs::default();
        let held = run(&prompt, &names, &cands, &none, &counts, 2, false);
        assert!(held.doors.iter().all(|d| d.status != DoorStatus::Held));
        assert_eq!(
            select(&prompt, &names, &cands, &none, &counts, 2),
            explain(&prompt, &names, &cands, &none, &counts, 2).selection()
        );
    }

    #[test]
    fn a_door_in_several_segments_fires_once_with_the_most_specific_trigger() {
        let trig = vec!["gerf".to_string(), "gerf_slips".to_string()];
        let cands = [cand("d", "aaa", "v", "2026-01-01T00:00:00Z", &trig)];
        let prompt = format!("ask gerf\n\n{}", paste("aaaa", "gerf_slips is permanent"));
        let sel = select(
            &prompt,
            &NamesUs::default(),
            &cands,
            &HashSet::new(),
            &HashMap::new(),
            2,
        );
        assert_eq!(sel.fired.len(), 1);
        assert_eq!(
            (sel.fired[0].trigger.as_str(), sel.fired[0].origin),
            ("gerf_slips", "paste")
        );

        // Equal specificity: the earlier segment wins.
        let ex = explain_on(
            &format!("widget\n\n{}", paste("aaaa", "widget")),
            &["widget"],
            2,
        );
        assert_eq!(
            outcome(&ex),
            vec![("widget".into(), DoorStatus::Fired, "typed", 1)]
        );
        let ex = explain_on(
            &format!("{}\n\nwidget", paste("aaaa", "widget")),
            &["widget"],
            2,
        );
        assert_eq!(
            outcome(&ex),
            vec![("widget".into(), DoorStatus::Fired, "paste", 1)]
        );
    }

    #[test]
    fn already_fired_doors_are_neither_fired_nor_held() {
        let trig = vec!["fair haven".to_string()];
        let cands = [cand("d", "aaa", "v", "2026-01-01T00:00:00Z", &trig)];
        let fired: HashSet<(String, String)> = [("d".to_string(), "aaa".to_string())].into();
        let ex = explain(
            &paste("aaaa", WALDEN),
            &NamesUs::default(),
            &cands,
            &fired,
            &HashMap::new(),
            2,
        );
        assert!(ex.doors.is_empty());
    }
}
