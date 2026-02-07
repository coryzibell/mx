//! Content manipulation operations
//!
//! Pure functions for editing, appending, and prepending content.
//! These are backend-agnostic and operate on strings directly.

use anyhow::Result;

/// Result of a content edit operation
#[derive(Debug)]
pub struct ContentEditResult {
    pub new_content: String,
    pub replacements: usize,
}

/// Perform find-and-replace on content
///
/// # Arguments
/// * `body` - The content to edit
/// * `old_text` - Text to find
/// * `new_text` - Text to replace with
/// * `replace_all` - If true, replace all occurrences
/// * `nth` - If Some(n), replace only the nth occurrence (1-indexed)
///
/// # Returns
/// ContentEditResult with the new content and number of replacements
pub fn edit_content(
    body: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
    nth: Option<usize>,
) -> Result<ContentEditResult> {
    // Count occurrences
    let matches: Vec<_> = body.match_indices(old_text).collect();
    let match_count = matches.len();

    if match_count == 0 {
        anyhow::bail!("Text not found in content: {:?}", old_text);
    }

    // Determine replacement strategy
    let new_content = if replace_all {
        body.replace(old_text, new_text)
    } else if let Some(n) = nth {
        if n == 0 || n > match_count {
            anyhow::bail!(
                "Invalid occurrence number: {} (found {} matches)",
                n,
                match_count
            );
        }
        // Replace only the nth occurrence (1-indexed)
        let (idx, _) = matches[n - 1];
        let mut result = String::with_capacity(body.len() + new_text.len() - old_text.len());
        result.push_str(&body[..idx]);
        result.push_str(new_text);
        result.push_str(&body[idx + old_text.len()..]);
        result
    } else if match_count == 1 {
        // Single match - just replace it
        body.replace(old_text, new_text)
    } else {
        // Multiple matches without --replace-all or --nth
        anyhow::bail!(
            "Found {} matches for {:?}. Use --replace-all to replace all, or --nth N to replace a specific occurrence.",
            match_count,
            old_text
        );
    };

    let replacements = if replace_all { match_count } else { 1 };

    Ok(ContentEditResult {
        new_content,
        replacements,
    })
}

/// Append content to existing content
///
/// # Arguments
/// * `existing` - The existing content (may be empty or None)
/// * `new_content` - Content to append
///
/// # Returns
/// The combined content
pub fn append_content(existing: Option<&str>, new_content: &str) -> String {
    match existing {
        Some(existing_content) if !existing_content.is_empty() => {
            format!("{}\n{}", existing_content, new_content)
        }
        _ => new_content.to_string(),
    }
}

/// Prepend content to existing content
///
/// # Arguments
/// * `existing` - The existing content (may be empty or None)
/// * `new_content` - Content to prepend
///
/// # Returns
/// The combined content
pub fn prepend_content(existing: Option<&str>, new_content: &str) -> String {
    match existing {
        Some(existing_content) if !existing_content.is_empty() => {
            format!("{}\n{}", new_content, existing_content)
        }
        _ => new_content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_single_match() {
        let result = edit_content("hello world", "world", "rust", false, None).unwrap();
        assert_eq!(result.new_content, "hello rust");
        assert_eq!(result.replacements, 1);
    }

    #[test]
    fn test_edit_multiple_matches_fails_without_flag() {
        let result = edit_content("foo bar foo", "foo", "baz", false, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_edit_replace_all() {
        let result = edit_content("foo bar foo", "foo", "baz", true, None).unwrap();
        assert_eq!(result.new_content, "baz bar baz");
        assert_eq!(result.replacements, 2);
    }

    #[test]
    fn test_edit_nth_occurrence() {
        let result = edit_content("foo bar foo baz foo", "foo", "qux", false, Some(2)).unwrap();
        assert_eq!(result.new_content, "foo bar qux baz foo");
        assert_eq!(result.replacements, 1);
    }

    #[test]
    fn test_edit_nth_out_of_bounds() {
        let result = edit_content("foo bar", "foo", "baz", false, Some(5));
        assert!(result.is_err());
    }

    #[test]
    fn test_edit_not_found() {
        let result = edit_content("hello world", "rust", "python", false, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_append_to_existing() {
        let result = append_content(Some("existing"), "new");
        assert_eq!(result, "existing\nnew");
    }

    #[test]
    fn test_append_to_empty() {
        let result = append_content(Some(""), "new");
        assert_eq!(result, "new");
    }

    #[test]
    fn test_append_to_none() {
        let result = append_content(None, "new");
        assert_eq!(result, "new");
    }

    #[test]
    fn test_prepend_to_existing() {
        let result = prepend_content(Some("existing"), "new");
        assert_eq!(result, "new\nexisting");
    }

    #[test]
    fn test_prepend_to_empty() {
        let result = prepend_content(Some(""), "new");
        assert_eq!(result, "new");
    }

    #[test]
    fn test_prepend_to_none() {
        let result = prepend_content(None, "new");
        assert_eq!(result, "new");
    }

    // ========== EDGE CASE TESTS (Diffi's requests + bonus chaos) ==========

    #[test]
    fn test_edit_empty_old_text() {
        // Empty old_text finds matches everywhere and requires --replace-all
        let result = edit_content("hello world", "", "x", false, None);
        // Multiple matches (one at every position) - should error without replace-all
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Found"));
    }

    #[test]
    fn test_edit_empty_old_text_produces_chaos() {
        // Empty string matches at every position between characters
        // This is technically valid but probably not what users want
        let result = edit_content("hi", "", "x", true, None).unwrap();
        // Matches: |h|i| = "x" + "h" + "x" + "i" + "x"
        assert_eq!(result.new_content, "xhxix");
        assert_eq!(result.replacements, 3); // One before, between, and after each char
    }

    #[test]
    fn test_edit_new_text_contains_old_text_no_recursion() {
        // new_text contains old_text - should NOT recurse
        let result = edit_content("foo", "foo", "foobar", false, None).unwrap();
        assert_eq!(result.new_content, "foobar");
        assert_eq!(result.replacements, 1);
        // Should NOT become "foobarbar" from recursive replacement
    }

    #[test]
    fn test_edit_replace_all_new_contains_old() {
        // With replace_all, new_text containing old_text should still not recurse
        let result = edit_content("foo bar foo", "foo", "foo_new", true, None).unwrap();
        assert_eq!(result.new_content, "foo_new bar foo_new");
        assert_eq!(result.replacements, 2);
    }

    #[test]
    fn test_edit_unicode_basic() {
        // Basic unicode replacement - should work fine
        let result = edit_content("Hello 世界", "世界", "World", false, None).unwrap();
        assert_eq!(result.new_content, "Hello World");
        assert_eq!(result.replacements, 1);
    }

    #[test]
    fn test_edit_unicode_emoji() {
        // Emoji are multi-byte - test replacing them
        let result = edit_content("I ❤️ Rust", "❤️", "love", false, None).unwrap();
        assert_eq!(result.new_content, "I love Rust");
    }

    #[test]
    fn test_edit_unicode_with_combining_characters() {
        // Combining characters (e.g., é = e + combining acute)
        // This tests if match_indices handles grapheme clusters correctly
        let text = "café"; // é might be one char or e + combining
        let result = edit_content(text, "é", "e", false, None).unwrap();
        assert!(result.new_content == "cafe");
    }

    #[test]
    fn test_edit_unicode_boundary_safety() {
        // match_indices should never split multi-byte chars
        // because it works on &str which is UTF-8 aware
        let text = "foo世bar";
        let result = edit_content(text, "世", "界", false, None).unwrap();
        assert_eq!(result.new_content, "foo界bar");
    }

    #[test]
    fn test_edit_nth_equals_one() {
        // nth=1 should replace first occurrence
        let result = edit_content("a b a", "a", "x", false, Some(1)).unwrap();
        assert_eq!(result.new_content, "x b a");
        assert_eq!(result.replacements, 1);
    }

    #[test]
    fn test_edit_nth_equals_max() {
        // nth=max should replace last occurrence
        let result = edit_content("a b a c a", "a", "x", false, Some(3)).unwrap();
        assert_eq!(result.new_content, "a b a c x");
        assert_eq!(result.replacements, 1);
    }

    #[test]
    fn test_edit_nth_beyond_max() {
        // nth beyond available matches should error
        let result = edit_content("a b a", "a", "x", false, Some(10));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid occurrence number")
        );
    }

    #[test]
    fn test_edit_nth_zero() {
        // nth=0 is invalid (1-indexed)
        let result = edit_content("a b a", "a", "x", false, Some(0));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid occurrence number")
        );
    }

    #[test]
    fn test_edit_very_large_content() {
        // Test with large content to ensure no panics/memory issues
        let large_text = "x".repeat(1_000_000);
        let result = edit_content(&large_text, "x", "y", false, Some(500_000)).unwrap();
        assert_eq!(result.new_content.chars().filter(|&c| c == 'y').count(), 1);
        assert_eq!(result.new_content.len(), 1_000_000);
    }

    #[test]
    fn test_edit_very_large_replace_all() {
        // Replace all in large content
        let large_text = "xy".repeat(100_000);
        let result = edit_content(&large_text, "x", "z", true, None).unwrap();
        assert_eq!(result.replacements, 100_000);
        assert!(result.new_content.starts_with("zy"));
    }

    #[test]
    fn test_append_to_empty_string_not_none() {
        // Some("") vs None - both should behave the same
        let result1 = append_content(Some(""), "new");
        let result2 = append_content(None, "new");
        assert_eq!(result1, result2);
        assert_eq!(result1, "new");
    }

    #[test]
    fn test_prepend_to_empty_string_not_none() {
        // Some("") vs None - both should behave the same
        let result1 = prepend_content(Some(""), "new");
        let result2 = prepend_content(None, "new");
        assert_eq!(result1, result2);
        assert_eq!(result1, "new");
    }

    #[test]
    fn test_edit_newlines_in_content() {
        // Multi-line content replacement
        let text = "line1\nline2\nline3";
        let result = edit_content(text, "line2", "REPLACED", false, None).unwrap();
        assert_eq!(result.new_content, "line1\nREPLACED\nline3");
    }

    #[test]
    fn test_edit_replace_with_empty_string() {
        // Replacing with empty string (deletion)
        let result = edit_content("foo bar baz", "bar ", "", false, None).unwrap();
        assert_eq!(result.new_content, "foo baz");
    }

    #[test]
    fn test_edit_old_text_longer_than_content() {
        // old_text longer than entire body
        let result = edit_content("hi", "hello world", "x", false, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_edit_overlapping_matches_replace_all() {
        // Overlapping matches - str::replace handles this correctly
        // "aaa" contains "aa" twice (overlapping), but replace only sees non-overlapping
        let result = edit_content("aaaa", "aa", "b", true, None).unwrap();
        assert_eq!(result.new_content, "bb"); // Replaces positions 0-2 and 2-4
        assert_eq!(result.replacements, 2);
    }

    #[test]
    fn test_edit_with_regex_special_chars() {
        // old_text contains chars that would be regex special - should treat literally
        let result = edit_content("a.b", ".", "X", false, None).unwrap();
        assert_eq!(result.new_content, "aXb");
    }

    #[test]
    fn test_edit_whitespace_only() {
        // Replacing whitespace
        let result = edit_content("a b  c", "  ", "_", false, None).unwrap();
        assert_eq!(result.new_content, "a b_c");
    }

    #[test]
    fn test_append_unicode() {
        let result = append_content(Some("Hello"), "世界");
        assert_eq!(result, "Hello\n世界");
    }

    #[test]
    fn test_prepend_unicode() {
        let result = prepend_content(Some("World"), "世界");
        assert_eq!(result, "世界\nWorld");
    }

    #[test]
    fn test_append_with_existing_newlines() {
        // If existing content has trailing newline
        let result = append_content(Some("line1\n"), "line2");
        assert_eq!(result, "line1\n\nline2"); // Double newline
    }

    #[test]
    fn test_edit_case_sensitive() {
        // Should be case-sensitive by default
        let result = edit_content("Hello World", "hello", "Hi", false, None);
        assert!(result.is_err()); // "hello" != "Hello"
    }

    #[test]
    fn test_edit_with_null_bytes() {
        // Null bytes in content (binary data edge case)
        let text = "foo\0bar";
        let result = edit_content(text, "\0", "_", false, None).unwrap();
        assert_eq!(result.new_content, "foo_bar");
    }
}
