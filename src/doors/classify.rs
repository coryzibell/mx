//! Whose text is this? A shape classifier that tells someone else's authored
//! prose (a book chapter, an article, a paper) from text that is ours (notes,
//! drafts, chat, our own markdown).
//!
//! A port of the calibrated Python reference (`foreign.py`: `features()` +
//! `score()`), rule for rule. Python semantics are kept where they differ from
//! Rust's defaults: lengths are Unicode scalar values, `str.split()` and
//! `str.strip()` use Python's whitespace set (which adds U+001C..U+001F), and
//! `str.splitlines()` breaks on Python's line-boundary set. The two lookbehind
//! patterns the `regex` crate cannot express are matched by hand.
//!
//! Known divergence: the `regex` crate's `\w` includes combining marks, ZWJ and
//! fullwidth `＿` and excludes superscript digits, and its `İ`/`ı` case folding
//! differs from Python's. Parity holds on plain text.
//!
//! Pure and IO-free. Regexes compile once per process: the hook runs on every
//! prompt.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// Segments shorter than this, in Unicode scalar values, are not judged.
pub const GATE_CHARS: usize = 600;

/// Python's `str.isspace()`: Unicode White_Space plus the four C0 separators.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `\w` for a single character, on plain text.
fn py_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn py_strip(s: &str) -> &str {
    s.trim_matches(py_space)
}

/// `len(s.split())`.
fn word_count(s: &str) -> usize {
    s.split(py_space).filter(|w| !w.is_empty()).count()
}

/// Python's `str.splitlines()`: no trailing empty line, `\r\n` is one break.
fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !is_break {
            continue;
        }
        out.push(&s[start..i]);
        start = i + c.len_utf8();
        if c == '\r'
            && let Some(&(j, '\n')) = it.peek()
        {
            start = j + 1;
            it.next();
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `(?<![\w.])Q(?![\w.])`: a standalone capital Q.
fn standalone_q_count(t: &str) -> usize {
    let chars: Vec<char> = t.chars().collect();
    let blocks = |c: Option<&char>| c.is_some_and(|&c| py_word(c) || c == '.');
    (0..chars.len())
        .filter(|&i| {
            chars[i] == 'Q'
                && !blocks(i.checked_sub(1).and_then(|p| chars.get(p)))
                && !blocks(chars.get(i + 1))
        })
        .count()
}

/// `re.split(r'(?<=[.!?])["”’]?\s+', s)`.
fn split_sentences(s: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let byte_at = |k: usize| chars.get(k).map_or(s.len(), |&(b, _)| b);
    let mut out = Vec::new();
    let mut start = 0;
    let mut k = 1;
    while k < chars.len() {
        if matches!(chars[k - 1].1, '.' | '!' | '?') {
            let mut j = k;
            if matches!(chars[j].1, '"' | '\u{201d}' | '\u{2019}') {
                j += 1;
            }
            let ws = j;
            while j < chars.len() && py_space(chars[j].1) {
                j += 1;
            }
            if j > ws {
                out.push(&s[start..byte_at(k)]);
                start = byte_at(j);
                k = j + 1;
                continue;
            }
        }
        k += 1;
    }
    out.push(&s[start..]);
    out
}

/// `statistics.median`: the mean of the two middle values for even n.
fn median(mut v: Vec<usize>) -> f64 {
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2] as f64
    } else {
        (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0
    }
}

// `\s` is widened to Python's whitespace set everywhere it appears.
macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(PARA_SPLIT, r"\n[\s\x{1C}-\x{1F}]*\n");
re!(PROSE_END, r#"[.!?]["”’)*_]*$"#);
re!(
    CHAT,
    r"(?i)\b(lol|lmao|haha+|omg|tbh|idk|btw|okay so|ok so)\b|[\x{1F300}-\x{1FAFF}]"
);
re!(
    HOUSE,
    r"\bkv-\w{4,}|\bkn-[0-9a-f]{6,}|~/|`[^`\n]+`|\bWake \d+"
);
re!(
    CITE,
    r"\[Footnote \d+|\[\d+(?:[\s\x{1C}-\x{1F}]*[,–-][\s\x{1C}-\x{1F}]*\d+)*\]|\bet al\.|\bdoi:|\b10\.\d{4,9}/|[¹²³⁴⁵⁶⁷⁸⁹⁰]|\([A-Z][a-z]+(?: (?:and|&) [A-Z][a-z]+| et al\.)?,? \d{4}\)"
);
re!(
    HEAD,
    r"(?im)^[*_# ]*(chapter|book|part|section|prologue|epilogue|interlude|abstract|introduction)\b[^\n]{0,40}$|^[*_ ]*[IVXLC]{1,6}\.?[*_ ]*$|^\d+(\.\d+)*[\s\x{1C}-\x{1F}]+[A-Z][A-Za-z ]{2,40}$"
);
re!(EPI, r"^(?:[*_ ]{0,4}[—–]|[*_ ]{0,4}--)");
re!(FROM_BY, r"\b(from|by)\b");
re!(
    LIST,
    r"^[\s\x{1C}-\x{1F}]*([-*•+][\s\x{1C}-\x{1F}]|\d+[.)][\s\x{1C}-\x{1F}]|\|)"
);
re!(
    CODE,
    r"(?m)```|^[\s\x{1C}-\x{1F}]{4,}[^\s\x{1C}-\x{1F}].*[;{}][\s\x{1C}-\x{1F}]*$"
);
re!(
    LOGHDR,
    r"(?m)^[\s\x{1C}-\x{1F}]*\[\d{1,2}:\d{2}\]|^.{1,40} — \d{1,2}/\d{1,2}/\d{2,4}"
);

/// The words that mark text as ours, matched whole-word and case-insensitive,
/// plus the standalone-`Q` rule, which is always on.
///
/// A parameter, not a list in the source: the repo is public and the names are
/// not.
#[derive(Debug, Clone, Default)]
pub struct NamesUs {
    re: Option<Regex>,
}

impl NamesUs {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let alts: Vec<String> = names
            .into_iter()
            .map(|n| n.as_ref().trim().to_lowercase())
            .filter(|n| !n.is_empty())
            .map(|n| regex::escape(&n))
            .collect();
        if alts.is_empty() {
            return Self::default();
        }
        let re = Regex::new(&format!(r"(?i)\b({})\b", alts.join("|"))).ok();
        Self { re }
    }

    fn count(&self, t: &str) -> usize {
        self.re.as_ref().map_or(0, |re| re.find_iter(t).count()) + standalone_q_count(t)
    }
}

/// The measured shape of one text: `features()` in the reference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Features {
    pub chars: usize,
    pub paras: usize,
    pub prose_share: f64,
    pub dialogue: f64,
    pub median_para_words: f64,
    pub mean_sentence_words: f64,
    pub cite: usize,
    pub head: usize,
    pub epi: usize,
    pub list_share: f64,
    pub code: usize,
    pub log: usize,
    pub fam: usize,
    pub chat: usize,
    pub house: usize,
}

pub fn features(t: &str, names: &NamesUs) -> Features {
    let paras: Vec<&str> = PARA_SPLIT
        .split(t)
        .map(py_strip)
        .filter(|p| !p.is_empty())
        .collect();
    let all_lines = py_splitlines(t);
    let lines: Vec<&str> = all_lines
        .iter()
        .copied()
        .filter(|l| !py_strip(l).is_empty())
        .collect();
    let mut pw: Vec<usize> = paras.iter().map(|p| word_count(p)).collect();
    if pw.is_empty() {
        pw.push(0);
    }
    let prose: Vec<&str> = paras
        .iter()
        .copied()
        .filter(|p| word_count(p) >= 12 && PROSE_END.is_match(p))
        .collect();
    let joined = prose.join(" ");
    let sents: Vec<usize> = split_sentences(&joined)
        .into_iter()
        .map(word_count)
        .filter(|&n| n > 2)
        .collect();
    let epi = all_lines
        .iter()
        .take(25)
        .filter(|l| EPI.is_match(l) && FROM_BY.is_match(l))
        .count();
    let n_paras = paras.len().max(1) as f64;
    let dialogue = paras
        .iter()
        .filter(|p| {
            p.trim_start_matches(['*', '_', ' '])
                .starts_with(['\u{201c}', '"'])
        })
        .count() as f64
        / n_paras;
    let listed = lines
        .iter()
        .filter(|l| LIST.is_match(l) && word_count(l) < 25)
        .count();
    Features {
        chars: t.chars().count(),
        paras: paras.len(),
        prose_share: prose.len() as f64 / n_paras,
        dialogue,
        median_para_words: median(pw),
        mean_sentence_words: if sents.is_empty() {
            0.0
        } else {
            sents.iter().sum::<usize>() as f64 / sents.len() as f64
        },
        cite: CITE.find_iter(t).count(),
        head: HEAD.find_iter(t).count(),
        epi,
        list_share: listed as f64 / lines.len().max(1) as f64,
        code: CODE.find_iter(t).count(),
        log: LOGHDR.find_iter(t).count(),
        fam: names.count(t),
        chat: CHAT.find_iter(t).count(),
        house: HOUSE.find_iter(t).count(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    ForeignProse,
    Ours,
    Unsure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Side {
    #[serde(rename = "F")]
    Foreign,
    #[serde(rename = "O")]
    Ours,
}

/// One rule that fired, its weight, and the measurement that tripped it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Signal {
    pub name: &'static str,
    pub side: Side,
    pub weight: u32,
    pub value: f64,
}

impl Signal {
    /// `dialogue .39`, `long-sents 16.0`, `citations 3`.
    pub fn label(&self) -> String {
        let v = match self.name {
            "dialogue" | "continuous-prose" | "lists" => {
                let s = format!("{:.2}", self.value);
                s.strip_prefix('0').map(str::to_string).unwrap_or(s)
            }
            "long-paras" | "paras" | "long-sents" => format!("{:.1}", self.value),
            _ => format!("{}", self.value),
        };
        format!("{} {}", self.name, v)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Classified {
    pub verdict: Verdict,
    pub f: u32,
    pub o: u32,
    pub signals: Vec<Signal>,
    /// Under `GATE_CHARS`: not judged, reported as Unsure.
    pub gated: bool,
}

pub fn score(x: &Features) -> Classified {
    let mut signals = Vec::new();
    let mut add = |name, side, weight, value: f64| {
        signals.push(Signal {
            name,
            side,
            weight,
            value,
        })
    };
    use Side::{Foreign as F, Ours as O};
    if x.dialogue >= 0.25 && x.paras >= 6 {
        add("dialogue", F, 3, x.dialogue);
    }
    if x.prose_share >= 0.6 {
        add("continuous-prose", F, 1, x.prose_share);
    }
    if x.median_para_words >= 60.0 {
        add("long-paras", F, 2, x.median_para_words);
    } else if x.median_para_words >= 35.0 {
        add("paras", F, 1, x.median_para_words);
    }
    if x.mean_sentence_words >= 20.0 {
        add("long-sents", F, 2, x.mean_sentence_words);
    } else if x.mean_sentence_words >= 15.0 {
        add("long-sents", F, 1, x.mean_sentence_words);
    }
    if x.cite >= 2 {
        add("citations", F, 2, x.cite as f64);
    }
    if x.head >= 1 {
        add("heading", F, 1, x.head as f64);
    }
    if x.epi >= 1 {
        add("epigraph", F, 2, x.epi as f64);
    }
    if x.list_share >= 0.2 {
        add("lists", O, 3, x.list_share);
    }
    if x.code >= 2 {
        add("code", O, 3, x.code as f64);
    }
    if x.log >= 2 {
        add("chatlog", O, 3, x.log as f64);
    }
    if x.fam >= 1 {
        add("names-us", O, if x.fam < 3 { 2 } else { 3 }, x.fam as f64);
    }
    if x.chat >= 2 {
        add("chat-register", O, 2, x.chat as f64);
    }
    if x.house >= 2 {
        add("house-vocab", O, 2, x.house as f64);
    }
    let sum = |side| {
        signals
            .iter()
            .filter(|s| s.side == side)
            .map(|s| s.weight)
            .sum::<u32>()
    };
    let (f, o) = (sum(F), sum(O));
    let gated = x.chars < GATE_CHARS;
    let verdict = if gated {
        Verdict::Unsure
    } else if o >= 3 {
        Verdict::Ours
    } else if f >= 3 && o <= 1 {
        Verdict::ForeignProse
    } else {
        Verdict::Unsure
    };
    Classified {
        verdict,
        f,
        o,
        signals,
        gated,
    }
}

pub fn classify(text: &str, names: &NamesUs) -> Classified {
    score(&features(text, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_q_matches_like_the_lookaround() {
        for (t, n) in [
            ("Q", 1),
            ("Q, okay", 1),
            ("ask Q.", 0),
            ("Q.E.D.", 0),
            ("IQ test", 0),
            ("Qatar", 0),
            ("a Q_b", 0),
            ("(Q)", 1),
            ("Q Q Q", 3),
            ("éQ", 0),
            ("Q2", 0),
            ("hi Q!", 1),
            ("q", 0),
        ] {
            assert_eq!(standalone_q_count(t), n, "{t:?}");
        }
    }

    #[test]
    fn sentence_split_matches_the_lookbehind() {
        assert_eq!(
            split_sentences("One two. Three four! Five? Six"),
            vec!["One two.", "Three four!", "Five?", "Six"]
        );
        // The optional closing quote is consumed with the whitespace.
        assert_eq!(
            split_sentences("He said \"go.\" Then left."),
            vec!["He said \"go.", "Then left."]
        );
        assert_eq!(
            split_sentences("\u{201c}Yes.\u{201d}  No."),
            vec!["\u{201c}Yes.", "No."]
        );
        // A quote not followed by whitespace is no split at all.
        assert_eq!(split_sentences("a.\"b c"), vec!["a.\"b c"]);
        // No terminal punctuation, no split; punctuation at the end, no split.
        assert_eq!(split_sentences("a b c"), vec!["a b c"]);
        assert_eq!(split_sentences("a b."), vec!["a b."]);
        assert_eq!(split_sentences("a.   "), vec!["a.", ""]);
        assert_eq!(split_sentences(""), vec![""]);
        assert_eq!(split_sentences("é. ü"), vec!["é.", "ü"]);
    }

    #[test]
    fn python_string_helpers() {
        assert_eq!(
            py_splitlines("a\nb\r\nc\rd\u{2028}e\n"),
            vec!["a", "b", "c", "d", "e"]
        );
        assert_eq!(py_splitlines("a\n\nb"), vec!["a", "", "b"]);
        assert!(py_splitlines("").is_empty());
        assert_eq!(word_count("  one\ttwo\u{1f}three\u{a0}four "), 4);
        assert_eq!(median(vec![3, 1, 2]), 2.0);
        assert_eq!(median(vec![4, 1, 2, 3]), 2.5);
    }

    #[test]
    fn gate_is_six_hundred_scalar_values() {
        // 599 two-byte characters: over 600 bytes, under 600 chars.
        let t = "é".repeat(599);
        assert!(classify(&t, &NamesUs::default()).gated);
        let t = "é".repeat(600);
        assert!(!classify(&t, &NamesUs::default()).gated);
    }

    #[test]
    fn names_are_whole_word_and_case_insensitive() {
        let names = NamesUs::new(["Robin", " sam ", ""]);
        assert_eq!(names.count("robin and SAM, robins, samwise"), 2);
        assert_eq!(NamesUs::default().count("robin Q"), 1);
        assert_eq!(NamesUs::new(["a.b"]).count("a.b axb"), 1);
    }

    macro_rules! fixture {
        ($dir:literal, $name:literal) => {
            (
                $name,
                include_str!(concat!("../../tests/fixtures/doors/", $dir, "/", $name)),
            )
        };
    }

    /// Every committed fixture, with the verdict, scores and signals the
    /// Python reference gives it (`foreign.py`, names `robin,sam`).
    #[test]
    fn fixtures_match_the_reference() {
        use Verdict::*;
        type Case = (
            (&'static str, &'static str),
            Verdict,
            u32,
            u32,
            &'static [&'static str],
        );
        let cases: [Case; 16] = [
            // A footnote apparatus block: Unsure in the reference too.
            (
                fixture!("pd", "emerson-footnotes.txt"),
                Unsure,
                2,
                0,
                &["citations"],
            ),
            (
                fixture!("pd", "emerson.txt"),
                ForeignProse,
                7,
                0,
                &["continuous-prose", "long-paras", "long-sents", "citations"],
            ),
            (
                fixture!("pd", "gibbon.txt"),
                ForeignProse,
                4,
                0,
                &["long-paras", "long-sents"],
            ),
            (
                fixture!("pd", "montaigne.txt"),
                ForeignProse,
                5,
                0,
                &["dialogue", "long-sents"],
            ),
            (
                fixture!("pd", "on-liberty.txt"),
                ForeignProse,
                5,
                0,
                &["continuous-prose", "long-paras", "long-sents"],
            ),
            (
                fixture!("pd", "origin-of-species.txt"),
                ForeignProse,
                5,
                0,
                &["continuous-prose", "long-paras", "long-sents"],
            ),
            (
                fixture!("pd", "the-prince.txt"),
                ForeignProse,
                7,
                0,
                &["continuous-prose", "long-paras", "long-sents", "citations"],
            ),
            (
                fixture!("pd", "walden.txt"),
                ForeignProse,
                5,
                0,
                &["continuous-prose", "long-paras", "long-sents"],
            ),
            (
                fixture!("syn", "syn-article.txt"),
                ForeignProse,
                4,
                0,
                &["continuous-prose", "paras", "long-sents"],
            ),
            (
                fixture!("syn", "syn-chapter-opener.txt"),
                ForeignProse,
                6,
                0,
                &["dialogue", "heading", "epigraph"],
            ),
            (
                fixture!("syn", "syn-chatlog.txt"),
                Ours,
                2,
                8,
                &["long-paras", "chatlog", "names-us", "chat-register"],
            ),
            (fixture!("syn", "syn-cv.txt"), Ours, 0, 3, &["lists"]),
            (
                fixture!("syn", "syn-essay-to-q.txt"),
                Unsure,
                3,
                2,
                &["continuous-prose", "paras", "long-sents", "names-us"],
            ),
            // The known false-positive class (design §6.3): first-person essay
            // prose that never names us reads exactly like Montaigne or Walden.
            // Pinned so a change to it is a decision, not an accident.
            (
                fixture!("syn", "syn-essay-unaddressed.txt"),
                ForeignProse,
                4,
                0,
                &["continuous-prose", "long-paras", "long-sents"],
            ),
            (
                fixture!("syn", "syn-paper-abstract.txt"),
                ForeignProse,
                7,
                0,
                &["long-paras", "long-sents", "citations", "heading"],
            ),
            (
                fixture!("syn", "syn-paper-intro.txt"),
                ForeignProse,
                8,
                0,
                &[
                    "continuous-prose",
                    "long-paras",
                    "long-sents",
                    "citations",
                    "heading",
                ],
            ),
        ];
        let names = NamesUs::new(["robin", "sam"]);
        for ((file, text), verdict, f, o, signals) in cases {
            let c = classify(text, &names);
            let got: Vec<&str> = c.signals.iter().map(|s| s.name).collect();
            assert_eq!(
                (c.verdict, c.f, c.o, got.as_slice()),
                (verdict, f, o, signals),
                "{file}"
            );
        }
    }

    #[test]
    fn injected_names_make_the_chatlog_name_us() {
        let (_, log) = fixture!("syn", "syn-chatlog.txt");
        let names = |c: &Classified| c.signals.iter().find(|s| s.name == "names-us").cloned();
        // Unset: only the one standalone Q counts.
        let bare = classify(log, &NamesUs::default());
        assert_eq!(names(&bare).map(|s| (s.weight, s.value)), Some((2, 1.0)));
        let named = classify(log, &NamesUs::new(["robin", "sam"]));
        assert_eq!(names(&named).map(|s| (s.weight, s.value)), Some((3, 21.0)));
    }

    #[test]
    fn signal_labels_carry_the_measurement() {
        let s = |name, value| Signal {
            name,
            side: Side::Foreign,
            weight: 1,
            value,
        };
        assert_eq!(s("dialogue", 0.3913).label(), "dialogue .39");
        assert_eq!(s("long-sents", 16.0).label(), "long-sents 16.0");
        assert_eq!(s("epigraph", 1.0).label(), "epigraph 1");
    }
}
