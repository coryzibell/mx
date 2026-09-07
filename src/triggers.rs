//! Trigger matching engine, shared by every triggered-memory surface.
//!
//! The pipeline is `normalize → tokenize → (optionally stem) → contiguous-sequence
//! match`, described on [`match_triggers`]. It is a pure, exhaustively testable
//! function with no IO.
//!
//! Stemming is a caller decision. `mx doors` matches proper nouns — slaptop,
//! konkon, gerf, ayo- — where morphology is actively harmful: the English
//! Snowball stemmer folds Tagalog "ayos" to "ayo", so "ayos lang" would open the
//! `ayo-` door. Callers matching prose concepts ("diabetes" should fire on
//! "diabetic") pass `stem = true`.

use crate::knowledge::normalize_trigger;
use rust_stemmers::{Algorithm, Stemmer};

/// Tokenize a raw string for trigger matching.
///
/// Steps, in order, so author-time and match-time agree exactly:
///   1. `normalize_trigger` — NFC canonicalize + lowercase + whitespace-collapse
///      (the single shared normalizer; see `src/knowledge.rs`). Returns no tokens
///      for empty/whitespace-only input. NFC is canonical composition, NOT the
///      compatibility form NFKC, so compatibility-equivalent text does not fold:
///      fullwidth `ｋｏｎｋｏｎ` and the ligature `ﬁ` stay distinct from their
///      ASCII spellings and will not match a plain trigger. That is deliberate —
///      NFKC would also collapse things a proper-noun matcher wants kept apart.
///   2. Split on Unicode word boundaries: any run of non-alphanumeric characters
///      separates tokens. This is what makes matching **word-boundary** — "ai"
///      tokenizes "said" as `["said"]`, never exposing a bare "ai" token, so the
///      "ai" trigger cannot fire on "said". It is also why hyphenated triggers
///      split: `ayo-mirage` is the two-token phrase `[ayo, mirage]`, and the bare
///      `ayo-` is the single token `[ayo]`.
///   3. When `stem` is true, Porter/Snowball English-stem each token so
///      "diabetes" and "diabetic" collapse to one stem.
pub fn tokens(raw: &str, stem: bool) -> Vec<String> {
    let Some(normalized) = normalize_trigger(raw) else {
        return Vec::new();
    };

    let split = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| !tok.is_empty());

    if stem {
        let stemmer = Stemmer::create(Algorithm::English);
        split.map(|tok| stemmer.stem(tok).into_owned()).collect()
    } else {
        split.map(|tok| tok.to_string()).collect()
    }
}

/// Return true if `needle` appears as a contiguous subsequence of `haystack`.
///
/// Empty `needle` never matches (a trigger that tokenizes to nothing is inert).
/// Single-token needles match when that one token appears anywhere as a whole
/// token. Multi-token (phrase) needles require the tokens to appear **adjacent
/// and in order** — this is what makes "blood sugar" fire on "his blood sugar
/// today" but NOT on "sugar in blood" (order) or "blood pressure and sugar"
/// (contiguity).
fn contains_contiguous(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Match a single message against one entry's normalized trigger list.
///
/// Returns the subset of `triggers` that fire, in their stored order (so output
/// is deterministic). `triggers` are expected already-normalized (stored that
/// way at author time), but we re-tokenize them here with the same `stem` flag
/// used for the message — the only safe way to guarantee they line up.
pub fn match_triggers(message_tokens: &[String], triggers: &[String], stem: bool) -> Vec<String> {
    triggers
        .iter()
        .filter(|trig| contains_contiguous(message_tokens, &tokens(trig, stem)))
        .cloned()
        .collect()
}

/// A matched entry: the entry id plus which of its triggers fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    pub id: String,
    pub triggers_matched: Vec<String>,
}

/// Run the matcher over a message and a collection of (id, triggers) pairs.
///
/// Tokenizes the message ONCE, then checks every entry against the shared token
/// stream. Returns one `TriggerMatch` per entry that has at least one firing
/// trigger, preserving the input entry order.
pub fn match_entries<'a, I>(message: &str, entries: I, stem: bool) -> Vec<TriggerMatch>
where
    I: IntoIterator<Item = (&'a str, &'a [String])>,
{
    let message_tokens = tokens(message, stem);
    if message_tokens.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, triggers) in entries {
        let matched = match_triggers(&message_tokens, triggers, stem);
        if !matched.is_empty() {
            out.push(TriggerMatch {
                id: id.to_string(),
                triggers_matched: matched,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        tokens(s, true)
    }

    fn bare(s: &str) -> Vec<String> {
        tokens(s, false)
    }

    // ---- Word-boundary matching ----

    #[test]
    fn word_boundary_ai_does_not_fire_on_said_or_maintain() {
        let msg = toks("he said we should maintain it");
        // Single-word trigger "ai" must NOT match inside "said" or "maintain".
        assert!(match_triggers(&msg, &["ai".to_string()], true).is_empty());
    }

    #[test]
    fn word_boundary_ai_fires_as_whole_token() {
        let msg = toks("the ai is helpful");
        assert_eq!(
            match_triggers(&msg, &["ai".to_string()], true),
            vec!["ai".to_string()]
        );
    }

    // ---- Stemming ----

    #[test]
    fn stemming_diabetes_fires_on_diabetic() {
        let msg = toks("he is diabetic");
        assert_eq!(
            match_triggers(&msg, &["diabetes".to_string()], true),
            vec!["diabetes".to_string()]
        );
    }

    #[test]
    fn stemming_run_fires_on_running() {
        let msg = toks("she is running today");
        assert_eq!(
            match_triggers(&msg, &["run".to_string()], true),
            vec!["run".to_string()]
        );
    }

    // ---- Stemming OFF (the doors path) ----

    #[test]
    fn no_stem_diabetes_does_not_fire_on_diabetic() {
        let msg = bare("he is diabetic");
        assert!(match_triggers(&msg, &["diabetes".to_string()], false).is_empty());
    }

    #[test]
    fn no_stem_ayo_does_not_fire_on_tagalog_ayos() {
        // With stemming on, "ayos" folds to "ayo" and opens the ayo- door.
        assert_eq!(
            match_triggers(&toks("ayos lang"), &["ayo-".to_string()], true),
            vec!["ayo-".to_string()],
            "stemmed matching is exactly the false positive doors must avoid"
        );
        // With stemming off it does not.
        assert!(match_triggers(&bare("ayos lang"), &["ayo-".to_string()], false).is_empty());
    }

    #[test]
    fn no_stem_hyphen_trigger_is_a_bare_token_prefix() {
        // "ayo-" tokenizes to ["ayo"], so it fires on any ayo-* alias and on
        // bare "ayo" — the prefix behaviour, for free.
        assert_eq!(
            match_triggers(
                &bare("switched to ayo-mirage"),
                &["ayo-".to_string()],
                false
            ),
            vec!["ayo-".to_string()]
        );
        assert_eq!(
            match_triggers(&bare("AYO-LIGHT please"), &["ayo-".to_string()], false),
            vec!["ayo-".to_string()]
        );
    }

    #[test]
    fn no_stem_gerf_does_not_fire_on_gerfalcon() {
        assert!(
            match_triggers(&bare("the gerfalcon stooped"), &["gerf".to_string()], false).is_empty()
        );
        assert_eq!(
            match_triggers(&bare("ask gerf about it"), &["gerf".to_string()], false),
            vec!["gerf".to_string()]
        );
    }

    // ---- Phrase (contiguous-sequence) matching ----

    #[test]
    fn phrase_fires_on_contiguous_in_order() {
        let msg = toks("what is his blood sugar today");
        assert_eq!(
            match_triggers(&msg, &["blood sugar".to_string()], true),
            vec!["blood sugar".to_string()]
        );
    }

    #[test]
    fn phrase_does_not_fire_out_of_order() {
        let msg = toks("there is sugar in blood");
        assert!(match_triggers(&msg, &["blood sugar".to_string()], true).is_empty());
    }

    #[test]
    fn phrase_does_not_fire_when_not_contiguous() {
        let msg = toks("blood pressure and high sugar");
        assert!(match_triggers(&msg, &["blood sugar".to_string()], true).is_empty());
    }

    #[test]
    fn no_stem_doubled_noun_phrase_needs_adjacency() {
        let trig = vec!["garlic garlic".to_string()];
        assert_eq!(
            match_triggers(&bare("the garlic garlic is out"), &trig, false),
            trig
        );
        assert!(match_triggers(&bare("garlic and more garlic"), &trig, false).is_empty());
        // Double space + caps still normalize into the same two tokens.
        assert_eq!(match_triggers(&bare("Garlic  Garlic"), &trig, false), trig);
    }

    // ---- NFC (proves the shared normalizer carries through tokenization) ----

    #[test]
    fn nfc_precomposed_and_decomposed_cafe_match() {
        // Trigger stored precomposed, message arrives decomposed.
        let msg = toks("meet me at the cafe\u{0301} later");
        assert_eq!(
            match_triggers(&msg, &["caf\u{00e9}".to_string()], true),
            vec!["caf\u{00e9}".to_string()]
        );
    }

    #[test]
    fn nfc_holds_with_stemming_off() {
        let msg = bare("meet me at the cafe\u{0301} later");
        assert_eq!(
            match_triggers(&msg, &["caf\u{00e9}".to_string()], false),
            vec!["caf\u{00e9}".to_string()]
        );
    }

    // ---- match_entries integration ----

    #[test]
    fn match_entries_returns_per_entry_matched_triggers() {
        let brad = vec!["brad".to_string(), "blood sugar".to_string()];
        let drew = vec!["drew".to_string()];
        let entries: Vec<(&str, &[String])> =
            vec![("kn-brad", brad.as_slice()), ("kn-drew", drew.as_slice())];
        let matches = match_entries("can you check brad's blood sugar?", entries, true);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "kn-brad");
        assert_eq!(
            matches[0].triggers_matched,
            vec!["brad".to_string(), "blood sugar".to_string()]
        );
    }

    #[test]
    fn match_entries_empty_message_matches_nothing() {
        let trig = vec!["brad".to_string()];
        let entries: Vec<(&str, &[String])> = vec![("kn-brad", trig.as_slice())];
        assert!(match_entries("   ", entries, true).is_empty());
    }
}
