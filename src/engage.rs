//! Fuzzy phrase matching for the wake ritual.
//!
//! This module was the interactive `engage` ritual: a terminal walk through
//! the cascade with a three-attempt hint ladder. The wake ritual is one guess
//! from the title now, with no hints and no interactive mode, and none of that
//! surface had a subcommand or a handler reaching it. What survives is the
//! matcher the ritual still calls.

/// Fuzzy matching result
pub enum MatchResult {
    Exact,   // Perfect match
    Close,   // Levenshtein within 20%
    Partial, // 50%+ key words match
    Wrong,   // No meaningful overlap
}

/// Fuzzy match input against expected phrase
pub fn fuzzy_match(input: &str, expected: &str) -> MatchResult {
    let input_norm = normalize(input);
    let expected_norm = normalize(expected);

    // `normalize` strips every non-alphanumeric character, so a blank input
    // and a punctuation-only phrase both collapse to "" and would otherwise
    // compare Exact. Nothing meaningful can match nothing.
    if input_norm.is_empty() || expected_norm.is_empty() {
        return MatchResult::Wrong;
    }

    // Exact match
    if input_norm == expected_norm {
        return MatchResult::Exact;
    }

    // Levenshtein distance (close enough). Both sides are measured in
    // CHARACTERS: `levenshtein` counts character edits, so dividing by a byte
    // length would inflate the denominator 2-4x for non-ASCII text and widen
    // the 0.8 tolerance until a half-wrong guess counted as close.
    let distance = levenshtein(&input_norm, &expected_norm);
    let max_len = input_norm
        .chars()
        .count()
        .max(expected_norm.chars().count());
    let similarity = 1.0 - (distance as f64 / max_len as f64);

    if similarity >= 0.8 {
        return MatchResult::Close;
    }

    // Word-based matching
    let input_words = extract_key_words(&input_norm);
    let expected_words = extract_key_words(&expected_norm);

    if !expected_words.is_empty() {
        let matches = input_words
            .iter()
            .filter(|w| expected_words.contains(w))
            .count();
        let match_ratio = matches as f64 / expected_words.len() as f64;

        if match_ratio >= 0.5 {
            return MatchResult::Partial;
        }
    }

    MatchResult::Wrong
}

/// Normalize text for comparison
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Extract key words (filter stop words)
fn extract_key_words(text: &str) -> Vec<String> {
    let stop_words = ["the", "a", "an", "is", "are", "i", "you", "we"];
    text.split_whitespace()
        .filter(|w| !stop_words.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Compute Levenshtein distance
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for i in 1..=a_len {
        curr_row[0] = i;

        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };

            curr_row[j] = (prev_row[j] + 1)
                .min(curr_row[j - 1] + 1)
                .min(prev_row[j - 1] + cost);
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    // =====================================================================
    // Fuzzy match tests with multi-byte characters
    // =====================================================================

    #[test]
    fn test_fuzzy_match_exact_with_emoji() {
        let phrase = "\u{1F41F} fish \u{1F41F}";
        match fuzzy_match(phrase, phrase) {
            MatchResult::Exact => {} // expected
            _ => panic!("Expected exact match for identical emoji strings"),
        }
    }

    #[test]
    fn test_fuzzy_match_with_cjk() {
        let phrase = "\u{4E16}\u{754C}\u{4F60}\u{597D}";
        match fuzzy_match(phrase, phrase) {
            MatchResult::Exact => {} // expected
            _ => panic!("Expected exact match for identical CJK strings"),
        }
    }

    // =====================================================================
    // The tolerance is measured in characters on both sides.
    //
    // `levenshtein` counts character edits; `max_len` used to count bytes.
    // For 3-byte text the denominator was 3x too large, so the 0.8 threshold
    // silently widened until a guess with half its characters wrong scored
    // 0.83 and came back Close. Every authored phrase is compared now and the
    // verdict is written to the guess log, so the encoding of a phrase must
    // not change what counts as a match.
    // =====================================================================

    /// Same edit ratio, different encodings, same verdict.
    #[test]
    fn tolerance_does_not_widen_for_multibyte_text() {
        // 5 of 10 characters differ: half wrong in any encoding.
        let ascii = fuzzy_match("abcdeqrstu", "abcdefghij");
        let kana = fuzzy_match(
            "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{3055}\u{3057}\u{3059}\u{305B}\u{305D}",
            "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{304B}\u{304D}\u{304F}\u{3051}\u{3053}",
        );
        assert!(
            matches!(ascii, MatchResult::Partial | MatchResult::Wrong),
            "precondition: half-wrong ASCII is not Close"
        );
        assert!(
            matches!(kana, MatchResult::Partial | MatchResult::Wrong),
            "a half-wrong multibyte guess was scored Close"
        );
    }

    /// One edit in ten characters is Close whatever the encoding.
    #[test]
    fn a_near_miss_is_close_in_either_encoding() {
        assert!(matches!(
            fuzzy_match("abcdefghix", "abcdefghij"),
            MatchResult::Close
        ));
        assert!(matches!(
            fuzzy_match(
                "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{304B}\u{304D}\u{304F}\u{3051}\u{3055}",
                "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{304B}\u{304D}\u{304F}\u{3051}\u{3053}",
            ),
            MatchResult::Close
        ));
    }

    /// Text that carries no alphanumeric content normalizes to the empty
    /// string. Two of those used to compare Exact.
    #[test]
    fn nothing_never_matches_nothing() {
        for (input, expected) in [
            ("", ""),
            ("   ", "\u{2014}"),
            ("\u{2014}", "   "),
            ("...", "???"),
            ("", "alpha"),
            ("alpha", "---"),
        ] {
            assert!(
                matches!(fuzzy_match(input, expected), MatchResult::Wrong),
                "{input:?} vs {expected:?} was scored as a match"
            );
        }
    }
}
