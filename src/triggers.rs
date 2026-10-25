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
//!
//! [`Grammar`] is the doors-only match grammar (`p + q`, `prefix*`, `*suffix`).
//! It sits beside the plain pipeline and never changes it: a trigger with no
//! standalone `+` and no `*` is plain and goes through [`match_triggers`].

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

/// Fewest literal characters beside a wildcard's `*`.
pub const MIN_WILDCARD_CHARS: usize = 5;

/// One position in a grammar phrase: it matches exactly one message token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pat {
    Exact(String),
    /// `prefix*`: the token starts with this.
    Prefix(String),
    /// `*suffix`: the token ends with this.
    Suffix(String),
}

impl Pat {
    fn matches(&self, token: &str) -> bool {
        match self {
            Pat::Exact(t) => token == t,
            Pat::Prefix(p) => fold_sigma(token).starts_with(&*fold_sigma(p)),
            Pat::Suffix(s) => fold_sigma(token).ends_with(&*fold_sigma(s)),
        }
    }

    fn form(&self) -> String {
        match self {
            Pat::Exact(t) => t.clone(),
            Pat::Prefix(p) => format!("{p}*"),
            Pat::Suffix(s) => format!("*{s}"),
        }
    }
}

/// `ς` as `σ`. Lowercasing writes a final sigma wherever `Σ` ends a word, and a
/// wildcard literal ends where its `*` is, so `ΣΟΦΟΣ*` lowercases to `σοφος*`
/// while the `σοφοστες` it should hit carries a medial `σ`.
fn fold_sigma(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains('ς') {
        s.replace('ς', "σ").into()
    } else {
        s.into()
    }
}

/// A doors trigger written in the match grammar: `p + q` sides that must all
/// match in one token stream, any order, each a contiguous phrase whose words
/// may carry one leading or trailing `*`. A lone phrase is one side.
///
/// Doors only, and never stemmed. A trigger with no ` + ` and no `*` is not a
/// grammar trigger at all and keeps going through [`match_triggers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grammar {
    pub sides: Vec<Vec<Pat>>,
}

/// Why a trigger string is not valid grammar. Each names the rule it broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarError {
    EmptySide,
    SeveralStars(String),
    MidStar(String),
    NotAlphanumeric(String),
    ShortWildcard { word: String, chars: usize },
}

impl GrammarError {
    /// The rule broken. `name_word` false leaves out the offending word, for a
    /// one-word trigger the caller already shows whole.
    pub fn rule(&self, name_word: bool) -> String {
        let (word, rest) = match self {
            GrammarError::EmptySide => {
                return "has an empty side of `+`: every side of `p + q` needs a phrase".into();
            }
            GrammarError::SeveralStars(w) => (
                w,
                "more than one `*`: a wildcard word takes exactly one".to_string(),
            ),
            GrammarError::MidStar(w) => (
                w,
                "`*` mid-word: a wildcard goes at the start or the end of a word".to_string(),
            ),
            GrammarError::NotAlphanumeric(w) => (
                w,
                "something besides letters and digits: a wildcard word is letters and digits plus one `*`"
                    .to_string(),
            ),
            GrammarError::ShortWildcard { word, chars } => (
                word,
                format!("{chars} literal characters: a wildcard needs at least {MIN_WILDCARD_CHARS}"),
            ),
        };
        if name_word {
            format!("has {word:?} with {rest}")
        } else {
            format!("has {rest}")
        }
    }
}

impl std::fmt::Display for GrammarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.rule(true))
    }
}

/// Parse a trigger as grammar. `Ok(None)` means it is a plain trigger: no
/// standalone `+` word and no `*` anywhere, so `c++` stays plain.
pub fn parse_grammar(trigger: &str) -> Result<Option<Grammar>, GrammarError> {
    let Some(normalized) = normalize_trigger(trigger) else {
        return Ok(None);
    };
    let words: Vec<&str> = normalized.split(' ').collect();
    if !normalized.contains('*') && !words.contains(&"+") {
        return Ok(None);
    }
    let mut sides = Vec::new();
    for side in words.split(|w| *w == "+") {
        if side.is_empty() {
            return Err(GrammarError::EmptySide);
        }
        let mut pats = Vec::new();
        for word in side {
            if word.contains('*') {
                pats.push(parse_wildcard(word)?);
            } else {
                pats.extend(tokens(word, false).into_iter().map(Pat::Exact));
            }
        }
        sides.push(pats);
    }
    Ok(Some(Grammar { sides }))
}

fn parse_wildcard(word: &str) -> Result<Pat, GrammarError> {
    if word.matches('*').count() > 1 {
        return Err(GrammarError::SeveralStars(word.to_string()));
    }
    let (lit, prefix) = match (word.strip_suffix('*'), word.strip_prefix('*')) {
        (Some(lit), _) => (lit, true),
        (_, Some(lit)) => (lit, false),
        _ => return Err(GrammarError::MidStar(word.to_string())),
    };
    if !lit.chars().all(char::is_alphanumeric) {
        return Err(GrammarError::NotAlphanumeric(word.to_string()));
    }
    let chars = lit.chars().count();
    if chars < MIN_WILDCARD_CHARS {
        return Err(GrammarError::ShortWildcard {
            word: word.to_string(),
            chars,
        });
    }
    let lit = lit.to_string();
    Ok(if prefix {
        Pat::Prefix(lit)
    } else {
        Pat::Suffix(lit)
    })
}

impl Grammar {
    /// True when every side matches somewhere in `message_tokens`.
    pub fn matches(&self, message_tokens: &[String]) -> bool {
        self.sides
            .iter()
            .all(|side| side_matches(message_tokens, side))
    }

    /// Message tokens the trigger spans, summed over its sides.
    pub fn span(&self) -> usize {
        self.sides.iter().map(Vec::len).sum()
    }

    /// What each side matches as, space-joined, wildcards keeping their `*`.
    pub fn side_forms(&self) -> Vec<String> {
        self.sides
            .iter()
            .map(|side| side.iter().map(Pat::form).collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// The sides' forms joined with ` + `.
    pub fn form(&self) -> String {
        self.side_forms().join(" + ")
    }
}

fn side_matches(haystack: &[String], side: &[Pat]) -> bool {
    if side.is_empty() || side.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(side.len())
        .any(|w| w.iter().zip(side).all(|(tok, pat)| pat.matches(tok)))
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

    // ---- Grammar: parsing ----

    fn g(s: &str) -> Grammar {
        parse_grammar(s).unwrap().unwrap()
    }

    fn ex(s: &str) -> Pat {
        Pat::Exact(s.to_string())
    }

    #[test]
    fn grammar_plain_triggers_are_not_grammar() {
        for plain in [
            "zorbel",
            "zorbel quuxer",
            "c++",
            "c++ tips",
            "zim & zam",
            "blip-blop",
            "a+ grade",
            "study +blarnia",
            "!",
            "",
        ] {
            assert_eq!(parse_grammar(plain), Ok(None), "{plain:?}");
        }
    }

    #[test]
    fn grammar_and_splits_into_sides() {
        assert_eq!(
            g("Zorbel + quuxer  pod + frim_bo"),
            Grammar {
                sides: vec![
                    vec![ex("zorbel")],
                    vec![ex("quuxer"), ex("pod")],
                    vec![ex("frim"), ex("bo")],
                ]
            }
        );
        // `c++` inside a side is the plain token `c`, not an operator.
        assert_eq!(
            g("c++ + zorbel").sides,
            vec![vec![ex("c")], vec![ex("zorbel")]]
        );
    }

    #[test]
    fn grammar_wildcards_parse_as_prefix_and_suffix() {
        assert_eq!(
            g("Quuxer* pod *Flimbing").sides,
            vec![vec![
                Pat::Prefix("quuxer".to_string()),
                ex("pod"),
                Pat::Suffix("flimbing".to_string()),
            ]]
        );
        // Exactly five literal characters is enough.
        assert_eq!(g("glarp*").sides, vec![vec![Pat::Prefix("glarp".into())]]);
        // Counted in characters after NFC, not bytes: decomposed input composes.
        assert_eq!(
            g("*zu\u{0308}rnig").sides,
            vec![vec![Pat::Suffix("z\u{00fc}rnig".into())]]
        );
        assert_eq!(g("k\u{00e4}tt\u{00f6}*").span(), 1);
    }

    #[test]
    fn grammar_rejects_each_broken_rule() {
        for (raw, want) in [
            ("+", GrammarError::EmptySide),
            ("+ zorbel", GrammarError::EmptySide),
            ("zorbel +", GrammarError::EmptySide),
            ("zorbel + + quuxer", GrammarError::EmptySide),
            (
                "*",
                GrammarError::ShortWildcard {
                    word: "*".into(),
                    chars: 0,
                },
            ),
            ("**zorbel", GrammarError::SeveralStars("**zorbel".into())),
            ("*zorbel*", GrammarError::SeveralStars("*zorbel*".into())),
            ("zor*bel", GrammarError::MidStar("zor*bel".into())),
            ("*zorb-el", GrammarError::NotAlphanumeric("*zorb-el".into())),
            (
                "zorb*",
                GrammarError::ShortWildcard {
                    word: "zorb*".into(),
                    chars: 4,
                },
            ),
            (
                "quuxer + *flim",
                GrammarError::ShortWildcard {
                    word: "*flim".into(),
                    chars: 4,
                },
            ),
        ] {
            assert_eq!(parse_grammar(raw), Err(want), "{raw:?}");
        }
    }

    #[test]
    fn grammar_errors_name_the_rule() {
        let msg = |s: &str| parse_grammar(s).unwrap_err().to_string();
        assert!(msg("zorbel +").contains("empty side of `+`"));
        assert!(msg("**zorbel").contains("more than one `*`"));
        assert!(msg("zor*bel").contains("mid-word"));
        assert!(msg("*zorb-el").contains("letters and digits plus one `*`"));
        assert!(msg("zorb*").contains("at least 5"));
    }

    #[test]
    fn grammar_error_rule_can_leave_out_the_word() {
        let e = parse_grammar("fo*o").unwrap_err();
        assert_eq!(
            e.rule(true),
            "has \"fo*o\" with `*` mid-word: a wildcard goes at the start or the end of a word"
        );
        assert_eq!(
            e.rule(false),
            "has `*` mid-word: a wildcard goes at the start or the end of a word"
        );
        assert_eq!(e.to_string(), e.rule(true));
    }

    #[test]
    fn grammar_form_and_span() {
        let t = g("frim_bo + quuxer* + *flimbing pod");
        assert_eq!(t.form(), "frim bo + quuxer* + *flimbing pod");
        assert_eq!(t.side_forms(), vec!["frim bo", "quuxer*", "*flimbing pod"]);
        assert_eq!(t.span(), 5);
    }

    // ---- Grammar: matching ----

    #[test]
    fn grammar_and_matches_in_any_order() {
        let t = g("zorbel + quuxer pod");
        assert!(t.matches(&bare("the zorbel met a quuxer pod")));
        assert!(t.matches(&bare("a quuxer pod, then the zorbel")));
        // A side is still a contiguous phrase.
        assert!(!t.matches(&bare("zorbel quuxer and a pod")));
        // Every side is required.
        assert!(!t.matches(&bare("just a zorbel")));
    }

    #[test]
    fn grammar_wildcard_prefix_and_suffix_match_one_token() {
        let pre = g("quuxer*");
        assert!(pre.matches(&bare("so quuxering today")));
        assert!(pre.matches(&bare("a quuxer")));
        assert!(!pre.matches(&bare("an unquuxer")));
        let suf = g("*flimbing");
        assert!(suf.matches(&bare("much reflimbing")));
        assert!(suf.matches(&bare("FLIMBING")));
        assert!(!suf.matches(&bare("flimbings")));
        // Tokens split on punctuation, so the wildcard sees each part.
        assert!(suf.matches(&bare("co-flimbing")));
    }

    #[test]
    fn grammar_wildcard_inside_a_phrase_holds_its_position() {
        let t = g("pod quuxer* team");
        assert!(t.matches(&bare("the pod quuxering team")));
        assert!(!t.matches(&bare("the pod team quuxering")));
        assert!(!t.matches(&bare("the pod big quuxering team")));
    }

    #[test]
    fn grammar_unicode_wildcard_matches_across_nfc() {
        let t = g("z\u{00fc}rnig*");
        assert!(t.matches(&bare("ZU\u{0308}RNIGKEIT ahead")));
        assert!(!t.matches(&bare("zurnigkeit")));
    }

    #[test]
    fn grammar_wildcard_final_sigma_matches_medial_sigma() {
        // Upper-case authoring lowercases to a final ς before the `*`.
        for prefix in ["ΣΟΦΟΣ*", "σοφος*", "σοφοσ*"] {
            let t = g(prefix);
            assert!(t.matches(&bare("οι σοφοστες")), "{prefix:?}");
            assert!(t.matches(&bare("ΣΟΦΟΣΤΕΣ")), "{prefix:?}");
            assert!(t.matches(&bare("ο σοφος")), "{prefix:?}");
            assert!(!t.matches(&bare("σοφοζτες")), "{prefix:?}");
            assert!(!t.matches(&bare("σοφοτες")), "{prefix:?}");
        }
        for suffix in ["*ΦΙΛΟΣ", "*φιλος", "*φιλοσ"] {
            let t = g(suffix);
            assert!(t.matches(&bare("ο θεοφιλος")), "{suffix:?}");
            assert!(t.matches(&bare("ΘΕΟΦΙΛΟΣ")), "{suffix:?}");
            assert!(!t.matches(&bare("θεοφιλοζ")), "{suffix:?}");
            assert!(!t.matches(&bare("θεοφιλοι")), "{suffix:?}");
        }
        // The stored literal keeps what lowercasing wrote.
        assert_eq!(g("ΣΟΦΟΣ*").form(), "σοφος*");
    }
}
