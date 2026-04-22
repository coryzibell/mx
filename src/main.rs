#![allow(dead_code)]

mod cli;
mod codex;
mod commit;
mod content_ops;
mod convert;
mod display;
mod embeddings;
mod engage;
mod github;
mod handlers;
mod helpers;
mod index;
mod knowledge;
pub mod paths;
mod session;
mod state;
mod store;
mod surreal_db;
mod sync;
mod tensor;
mod types;
mod wake_chunk;
mod wake_ritual;
mod wake_token;

use anyhow::{Result, bail};
use clap::Parser;

use cli::*;
use handlers::*;

fn main() -> Result<()> {
    paths::emit_mx_home_note();

    let cli = Cli::parse();

    match cli.command {
        Commands::Memory { command } => handle_memory(command, cli.verbose),
        Commands::Commit {
            message,
            all,
            push,
            encode_only,
            title,
            body,
            show_encoded,
        } => {
            if encode_only {
                // PR-style encoding: encode title and body, print to stdout.
                // `--encode-only` is its own print path and is deliberately
                // left alone — its entire purpose is to emit encoded output.
                if let (Some(t), Some(b)) = (title, body) {
                    let encoded_message = commit::encode_commit_message(&t, &b)?;
                    println!("{}", encoded_message);
                } else {
                    // This shouldn't happen due to clap validation, but handle gracefully
                    bail!("--encode-only requires both --title and --body");
                }
            } else {
                // Normal commit workflow
                let msg =
                    message.ok_or_else(|| anyhow::anyhow!("message is required for commit"))?;
                commit::upload_commit(&msg, all, push, show_encoded)?;
            }
            Ok(())
        }
        Commands::Pr { command } => handle_pr(command),
        Commands::Sync { command } => sync::handle_sync(command),
        Commands::Github { command } => handle_github(command),
        Commands::Wiki { command } => handle_wiki(command),
        Commands::Session { command } => handle_session(command),
        Commands::Codex { command } => handle_codex(command),
        Commands::Convert { command } => handle_convert(command),
        Commands::Heartbeat { since, reset } => handle_heartbeat(since, reset),
        Commands::Log { count, full, args } => handle_log(count, full, args),
        Commands::State { command } => handle_state(command),
    }
}

#[cfg(test)]
mod tests {
    use crate::display::safe_truncate;

    #[test]
    fn test_safe_truncate_short_string() {
        // String shorter than limit - no truncation
        assert_eq!(safe_truncate("hello", 10), "hello");
    }

    #[test]
    fn test_safe_truncate_exact_length() {
        // String exactly at limit - no truncation
        assert_eq!(safe_truncate("hello", 5), "hello");
    }

    #[test]
    fn test_safe_truncate_long_string() {
        // String longer than limit - truncated with "..."
        assert_eq!(safe_truncate("hello world", 8), "hello...");
    }

    #[test]
    fn test_safe_truncate_emoji() {
        // Emoji (multi-byte UTF-8) - should not panic
        let emoji_string = "Hello! A fox for you 5 times";
        let result = safe_truncate(emoji_string, 15);
        // Should truncate by character count, not bytes
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 15); // 12 chars + 3 for "..."
    }

    #[test]
    fn test_safe_truncate_all_emoji() {
        // All emoji string - should handle gracefully
        let result = safe_truncate("aaaaaaaaaa", 5);
        assert_eq!(result, "aa...");
    }

    #[test]
    fn test_safe_truncate_empty() {
        // Empty string
        assert_eq!(safe_truncate("", 10), "");
    }

    #[test]
    fn test_safe_truncate_very_small_limit() {
        // Limit smaller than "..." length
        let result = safe_truncate("hello world", 3);
        // Should handle gracefully (saturating_sub prevents underflow)
        assert_eq!(result, "...");
    }

    // =====================================================================
    // Regression tests for unicode boundary panic fix (PR #162)
    //
    // These tests exercise the CALL SITES that previously used raw byte-index
    // slicing (&s[..N]) and would have panicked on multi-byte UTF-8 characters.
    // The fix replaced those with safe_truncate() which counts characters.
    // =====================================================================

    #[test]
    fn test_log_display_emoji_would_panic_at_byte_69() {
        // Regression: handle_log used `&display[..69]` which panics if byte 69
        // lands inside a multi-byte character.
        //
        // 73 fish emoji (U+1F41F, 4 bytes each) = 73 chars, 292 bytes.
        // Old code: `&display[..69]` slices at byte 69, inside the 18th emoji
        // (bytes 68..71). This panics with "byte index 69 is not a char boundary".
        let emoji_str: String = "\u{1F41F}".repeat(73);
        assert_eq!(emoji_str.chars().count(), 73);
        assert!(emoji_str.len() > 72);

        let result = safe_truncate(&emoji_str, 72);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 72);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_log_display_cjk_mixed_would_panic_at_byte_69() {
        // Mixed ASCII + CJK where byte 69 falls inside a CJK character.
        // 2 ASCII bytes + 24 CJK chars (72 bytes) = 26 chars, 74 bytes.
        // Old code: &display[..69] = byte 69 = 2 + 67, and 67 is NOT divisible
        // by 3, so byte 69 lands inside the 23rd CJK char. PANIC!
        let mut s = "ab".to_string();
        s.push_str(&"\u{4E16}".repeat(24));
        assert_eq!(s.chars().count(), 26);
        assert!(s.len() > 72);
        // Verify byte 69 is NOT a char boundary (the actual panic trigger)
        assert!(!s.is_char_boundary(69));

        let result = safe_truncate(&s, 72);
        // 26 chars < 72 limit, no truncation needed
        assert_eq!(result, s);
    }

    #[test]
    fn test_entry_summary_emoji_would_panic_at_byte_77() {
        // Regression: print_entry_summary used `&summary[..77]` which panics
        // if byte 77 lands inside a multi-byte character.
        //
        // 81 fish emoji = 81 chars, 324 bytes.
        // Old code: `&summary[..77]` = byte 77, inside the 20th emoji
        // (bytes 76..79). Panics with "byte index 77 is not a char boundary".
        let emoji_summary: String = "\u{1F41F}".repeat(81);
        assert_eq!(emoji_summary.chars().count(), 81);

        let result = safe_truncate(&emoji_summary, 80);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 80);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_entry_summary_cjk_would_panic_at_byte_77() {
        // 81 CJK chars (U+4E16) = 243 bytes.
        // Old code: &summary[..77]. 77 / 3 = 25.67 -> byte 77 is NOT on a
        // character boundary (char boundaries at 75, 78...). PANIC!
        let cjk_summary: String = "\u{4E16}".repeat(81);
        assert_eq!(cjk_summary.chars().count(), 81);
        // Verify byte 77 is indeed NOT a char boundary
        assert!(!cjk_summary.is_char_boundary(77));

        let result = safe_truncate(&cjk_summary, 80);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 80);
    }

    #[test]
    fn test_entry_summary_mixed_ascii_emoji_would_panic_at_byte_77() {
        // 75 ASCII + 6 emoji (4 bytes each) = 81 chars, 99 bytes.
        // Old code: &summary[..77] = byte 77 = 75 + 2, which is 2 bytes into
        // the first emoji. PANIC!
        let mut mixed = "x".repeat(75);
        for _ in 0..6 {
            mixed.push('\u{1F41F}');
        }
        assert_eq!(mixed.chars().count(), 81);
        // Verify byte 77 is NOT a char boundary (inside first emoji)
        assert!(!mixed.is_char_boundary(77));

        let result = safe_truncate(&mixed, 80);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 80);
    }

    #[test]
    fn test_fact_title_truncation_emoji_at_60_boundary() {
        // memory add --type uses safe_truncate(&body, 60) for fact titles.
        // 61 emoji = 244 bytes. Old byte-slicing would have panicked.
        let emoji_body: String = "\u{1F41F}".repeat(61);
        let result = safe_truncate(&emoji_body, 60);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 60);
    }

    #[test]
    fn test_recent_preview_cjk_at_60_boundary() {
        // memory recent uses safe_truncate(content, 60).
        // 61 CJK chars = 183 bytes.
        let long_cjk: String = "\u{4E16}".repeat(61);
        let result = safe_truncate(&long_cjk, 60);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 60);
    }
}
