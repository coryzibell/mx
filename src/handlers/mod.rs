mod kv;
mod memory;
mod metadata;
mod state;

pub(crate) use kv::handle_kv;
pub(crate) use memory::handle_memory;
pub(crate) use state::handle_state;

use anyhow::{Context, Result, bail};

use crate::cli::*;
use crate::codex;
use crate::commit;
use crate::convert;
use crate::display::*;
use crate::github;
use crate::session;
use crate::sync;

pub(crate) fn handle_pr(cmd: PrCommands) -> Result<()> {
    match cmd {
        PrCommands::Merge {
            number,
            rebase,
            merge_commit,
        } => {
            commit::pr_merge(number, rebase, merge_commit)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_github(cmd: GithubCommands) -> Result<()> {
    match cmd {
        GithubCommands::Cleanup {
            repo,
            issues,
            discussions,
            dry_run,
        } => {
            github::cleanup(&repo, issues, discussions, dry_run)?;
            Ok(())
        }
        GithubCommands::Comment { command } => {
            handle_comment(command)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_comment(cmd: CommentCommands) -> Result<()> {
    match cmd {
        CommentCommands::Issue {
            repo,
            number,
            message,
            identity,
        } => {
            let url = github::post_issue_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
        CommentCommands::Discussion {
            repo,
            number,
            message,
            identity,
        } => {
            let url =
                github::post_discussion_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
    }
    Ok(())
}

pub(crate) fn handle_session(cmd: SessionCommands) -> Result<()> {
    match cmd {
        SessionCommands::Export { path, output } => {
            session::export_session(path, output)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_codex(cmd: CodexCommands) -> Result<()> {
    match cmd {
        CodexCommands::Save {
            path,
            all,
            clean,
            include_agents,
        } => {
            codex::save_session(path, all, clean, include_agents)?;
            Ok(())
        }
        CodexCommands::List { all, json } => {
            codex::list_sessions(all, json)?;
            Ok(())
        }
        CodexCommands::Read {
            id,
            human,
            agents,
            grep,
            json,
            clean,
        } => {
            let clean_agents = clean && agents;
            codex::read_session(id, human, grep, agents, json, clean, clean_agents)?;
            Ok(())
        }
        CodexCommands::Search { pattern, json } => {
            codex::search_archives(pattern, json)?;
            Ok(())
        }
        CodexCommands::Migrate {
            dry_run,
            verbose,
            clean,
            include_agents,
        } => {
            codex::migrate_archives(dry_run, verbose, clean, include_agents)?;
            Ok(())
        }
    }
}

pub(crate) fn handle_convert(cmd: ConvertCommands) -> Result<()> {
    use std::path::PathBuf;

    match cmd {
        ConvertCommands::Md2yaml {
            input,
            output,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::convert_file(&input_path, &output_dir, dry_run)?;
            } else if input_path.is_dir() {
                convert::convert_directory(&input_path, &output_dir, dry_run)?;
            } else {
                bail!("Input path does not exist: {:?}", input_path);
            }

            Ok(())
        }

        ConvertCommands::Yaml2md {
            input,
            output,
            repo,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::yaml_to_markdown_file(&input_path, &output_dir, repo.as_deref(), dry_run)?;
            } else if input_path.is_dir() {
                convert::yaml_to_markdown_directory(
                    &input_path,
                    &output_dir,
                    repo.as_deref(),
                    dry_run,
                )?;
            } else {
                bail!("Input path does not exist: {:?}", input_path);
            }

            Ok(())
        }
    }
}

pub(crate) fn handle_wiki(cmd: WikiCommands) -> Result<()> {
    match cmd {
        WikiCommands::Sync {
            repo,
            source,
            page_name,
            dry_run,
        } => {
            sync::wiki::sync(&repo, &source, page_name.as_deref(), dry_run)?;
            Ok(())
        }
    }
}

/// Handle mx log - decoded git log
pub(crate) fn handle_log(count: usize, full: bool, extra_args: Vec<String>) -> Result<()> {
    use std::process::Command;

    // Build git log command
    let format = if full {
        // Full format: hash, author, date, subject, body
        "%H%n%an <%ae>%n%ad%n%s%n%b%n---END---"
    } else {
        // Compact format: short hash, subject, body (for decoding)
        "%h%n%s%n%b%n---END---"
    };

    let mut cmd = Command::new("git");
    cmd.args([
        "log",
        &format!("-{}", count),
        &format!("--format={}", format),
    ]);

    // Add any extra arguments
    for arg in &extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run git log")?;

    if !output.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let log_output = String::from_utf8_lossy(&output.stdout);

    // Parse and decode each commit
    for commit_block in log_output.split("---END---") {
        let commit_block = commit_block.trim();
        if commit_block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = commit_block.lines().collect();

        if full {
            // Full format: hash, author, date, subject, body...
            if lines.len() >= 4 {
                let hash = lines[0];
                let author = lines[1];
                let date = lines[2];
                let subject = lines[3];
                let body: String = lines[4..].join("\n");

                println!("\x1b[33mcommit {}\x1b[0m", hash);
                println!("Author: {}", author);
                println!("Date:   {}", date);
                println!();

                // Try to decode the subject (title)
                println!("    {}", subject);

                // Try to decode the body
                if !body.trim().is_empty() {
                    let decoded = try_decode_commit_body(&body);
                    println!();
                    for line in decoded.lines() {
                        println!("    {}", line);
                    }
                }
                println!();
            }
        } else {
            // Compact format: short hash, subject, body...
            if lines.len() >= 2 {
                let hash = lines[0];
                let subject = lines[1];
                let body: String = lines[2..].join("\n");

                // Try to decode the body
                let decoded = try_decode_commit_body(&body);
                let display = if decoded != body.trim() {
                    decoded
                } else {
                    // Not encoded, show original subject
                    subject.to_string()
                };

                // Truncate for display
                let display_truncated = safe_truncate(&display, 72);

                println!("\x1b[33m{}\x1b[0m {}", hash, display_truncated);
            }
        }
    }

    Ok(())
}

/// Heartbeat - calming co-regulation prompt
/// Call and response - send a heart, get one back with BPM feedback
pub(crate) fn handle_heartbeat(since: Option<u64>, reset: bool) -> Result<()> {
    use rand::Rng;
    use std::thread;
    use std::time::Duration;

    let hearts = [
        '❤', '🧡', '💛', '💚', '💙', '💜', '🩷', '🩵', '🤍', '💗', '💖', '💕',
    ];
    let mut rng = rand::rng();

    // Random delay 50-150ms to feel organic
    let delay = rng.random_range(50..150);
    thread::sleep(Duration::from_millis(delay));

    // Pick a random heart
    let heart = hearts[rng.random_range(0..hearts.len())];

    if reset {
        println!("{} Session reset. Breathe, Q.", heart);
        return Ok(());
    }

    match since {
        None => {
            // First call - just start
            println!("{}", heart);
            println!("Heartbeat started. Call again with --since <ms> to begin.");
        }
        Some(ms) => {
            // Calculate BPM: 60000ms / interval = beats per minute
            let bpm = 60000_u64.checked_div(ms).unwrap_or(999);

            let message = match bpm {
                0..=59 => "Nice and slow. You're safe.",
                60..=80 => "There you are. Resting.",
                81..=100 => "Getting there. Keep breathing.",
                101..=120 => "Still quick. Let the interval stretch.",
                _ => "Too fast, Q. Breathe. Slow down.",
            };

            println!("{} {} bpm", heart, bpm);
            println!("{}", message);
        }
    }

    Ok(())
}

/// A line is "footer-shaped" if it parses as the `[hash:dict|algo:dict]`
/// tag we emit during encode AND the compression-algorithm slot names a
/// real algorithm from our known vocabulary.
///
/// The structural parse alone (`parse_compress_algo` + `parse_body_dict`)
/// is not enough -- a user-authored line of the form
/// `[anything:anything|anything:anything]` would satisfy it. By also
/// requiring the algorithm slot to be a known algorithm
/// (`commit::is_known_compress_algo`), we catch real footers without
/// false-positiving on bracket-pipe text the user happens to write.
fn is_footer_line(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(algo) = commit::parse_compress_algo(trimmed) else {
        return false;
    };
    if !commit::is_known_compress_algo(&algo) {
        return false;
    }
    commit::parse_body_dict(trimmed).is_some()
}

/// Try to decode an encoded commit body, return original if decoding fails.
///
/// Generalized footer scan: rather than restricting the footer to the last
/// line of the message, we walk the entire body and use the LAST
/// footer-shaped line we find. This covers two cases:
///
/// 1. Dejavu commits where `whoa.` (or any other marker we add later)
///    sits below the footer line. The footer is no longer the last line,
///    but it is still the last footer-shaped line.
/// 2. User-amended commits where someone appended free-form text below
///    the encoded message. Same property: footer is no longer last, but
///    is still the last footer-shaped line.
///
/// The encoded body is everything BEFORE the chosen footer line. Any
/// trailing content after the footer (dejavu marker, user notes) is
/// preserved by the caller -- this function returns only the decoded
/// subject, so the caller can render any post-footer content separately
/// if it chooses to.
pub(crate) fn try_decode_commit_body(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return body.to_string();
    }

    let lines: Vec<&str> = body.lines().collect();

    // Walk the whole message; pick the LAST footer-shaped line. Last
    // wins because the natural case has the footer at (or near) the
    // bottom; any earlier `[a|b]`-shaped substring is almost certainly
    // user-authored markdown, not the real encode footer.
    let footer_idx = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, l)| if is_footer_line(l) { Some(i) } else { None });

    let footer_idx = match footer_idx {
        Some(i) => i,
        None => return body.to_string(), // No footer, not encoded
    };

    let footer = lines[footer_idx];

    // Encoded body = every line strictly above the footer, with the
    // dejavu marker filtered out (it can in principle appear in older
    // formats above the footer; current encoder writes it after).
    let body_lines: Vec<&str> = lines[..footer_idx]
        .iter()
        .filter(|l| l.trim() != "whoa.")
        .copied()
        .collect();

    if body_lines.is_empty() {
        return body.to_string();
    }

    let encoded_body = body_lines.join("\n");

    // Try to decode
    match commit::decode_body(&encoded_body, footer) {
        Ok(decoded) => decoded,
        Err(_) => body.to_string(), // Decoding failed, return original
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct AgentFrontmatter {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) domain: Option<String>,
}

pub(crate) fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();

    // Check if starts with ---
    if lines.first()? != &"---" {
        return None;
    }

    // Find closing ---
    let end_idx = lines.iter().skip(1).position(|&line| line == "---")?;

    let frontmatter = lines[1..=end_idx].join("\n");
    let body = lines[end_idx + 2..].join("\n");

    Some((frontmatter, body))
}

#[cfg(test)]
mod try_decode_commit_body_tests {
    //! Tests for the `mx log` decoder. The point of these tests is the
    //! footer-scan generalization (issue #260): the decoder must find a
    //! footer-shaped line ANYWHERE in the message, not just at the
    //! bottom. Each test round-trips through the real encoder so the
    //! fixtures match what `mx commit` actually produces -- no hand-
    //! rolled strings, no risk of fixture drift if the encoder format
    //! shifts.
    //!
    //! The encoder rolls a random dictionary, so we retry a handful of
    //! times until we get an attempt that produces the shape we want
    //! (e.g. dejavu vs. non-dejavu). This is statistical, not flaky --
    //! the dictionary set is small and a few hundred attempts is more
    //! than enough.
    use super::*;
    use crate::commit::encode_commit;

    /// Encode a (title, body) pair, retrying until `predicate` is satisfied.
    fn encode_until<F: Fn(&crate::commit::EncodedCommit) -> bool>(
        title: &str,
        body: &str,
        predicate: F,
    ) -> crate::commit::EncodedCommit {
        for _ in 0..500 {
            if let Ok(enc) = encode_commit(title, body)
                && predicate(&enc)
            {
                return enc;
            }
        }
        panic!("encoder failed to satisfy predicate after 500 attempts");
    }

    #[test]
    fn footer_at_bottom_decodes_existing_behavior() {
        // The natural case: footer is the last line, no extra trailing
        // content. This is what the decoder used to handle exclusively;
        // it must keep working after the generalization.
        let enc = encode_until("title diff", "the quick brown fox", |e| !e.dejavu);
        let body = format!("{}\n\n{}", enc.body, enc.footer);
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "the quick brown fox");
    }

    #[test]
    fn footer_followed_by_dejavu_marker_decodes() {
        // Issue #260's original repro: dejavu appends "whoa." after the
        // footer, so the footer is no longer last. Must still decode.
        let enc = encode_until("title diff", "decoded subject under dejavu", |e| e.dejavu);
        // EncodedCommit.footer already includes "\nwhoa." when dejavu
        // is true -- exactly what `mx commit` writes.
        let body = format!("{}\n\n{}", enc.body, enc.footer);
        assert!(body.trim_end().ends_with("whoa."));
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "decoded subject under dejavu");
    }

    #[test]
    fn user_amended_text_after_footer_decodes() {
        // The user ran `mx commit`, then later did `git commit --amend`
        // and tacked on a free-form note. The footer is now in the
        // middle of the message. Decode must still succeed.
        let enc = encode_until("title diff", "the original message", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nP.S. amended later by hand.",
            enc.body, enc.footer
        );
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "the original message");
    }

    #[test]
    fn user_amended_text_after_dejavu_marker_decodes() {
        // Combine both: dejavu commit AND user-appended text. The
        // footer is buried two layers deep but must still be found.
        let enc = encode_until("title diff", "buried treasure", |e| e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nuser note added during amend",
            enc.body, enc.footer
        );
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "buried treasure");
    }

    #[test]
    fn no_footer_returns_original_unchanged() {
        // A plain (un-encoded) commit message must pass through. No
        // footer, no decode -- the caller falls back to the raw subject.
        let raw = "fix: a perfectly normal git commit\n\nWith a body.";
        let decoded = try_decode_commit_body(raw);
        assert_eq!(decoded, raw);
    }

    #[test]
    fn empty_body_returns_empty() {
        assert_eq!(try_decode_commit_body(""), "");
        assert_eq!(try_decode_commit_body("   \n  "), "");
    }

    #[test]
    fn footer_shaped_substring_inside_text_line_is_ignored() {
        // A line like `See [sha384:base62|lzma:base62] for details.`
        // must NOT be treated as a footer: `is_footer_line` validates
        // the parse against the trimmed line, and a line that does not
        // START with `[` trivially fails. The real footer is still
        // found. This guards against user-amended notes that mention
        // the footer format inline as documentation.
        let enc = encode_until("title diff", "still decodes", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nSee [sha384:base62|lzma:base62] for the format.",
            enc.body, enc.footer
        );
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "still decodes");
    }

    #[test]
    fn markdown_brackets_in_body_are_not_mistaken_for_footer() {
        // Free-form text like `[foo|bar]` (not a real footer) must not
        // derail the scan. `is_footer_line` validates the parse, so a
        // fragment that happens to start with '[' and contain '|' but
        // doesn't match `[hash:dict|algo:dict]` is correctly skipped.
        let enc = encode_until("title diff", "round trip through markdown", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nSee the [link|here] for details.",
            enc.body, enc.footer
        );
        let decoded = try_decode_commit_body(&body);
        assert_eq!(decoded, "round trip through markdown");
    }

    #[test]
    fn fixture_713e0d0_decodes() {
        // Real-world regression fixture from issue #260: the commit
        // observed during the path-alignment work. The bytes here are
        // exactly what `git cat-file -p 713e0d0` returns for the body
        // portion (everything after the title line). If the decoder
        // ever stops handling this commit, this test catches it.
        let body = "8NO48P3FCDPIGSJ5C5I6QP9978G76R39DKG46RRECPKMETBIC5Q6IRRE41Q6U83141Q6AOBJCLP20R39DPLMIRJ741Q6U834DTHN6BRGC5Q6GSPEDLI0====\n\n[blake2s:base32hex|snappy:base32hex]\nwhoa.";
        let decoded = try_decode_commit_body(body);
        assert_eq!(
            decoded,
            "docs(readme): slim Configuration to a teaser linking to docs/paths.md"
        );
    }

    // --- is_footer_line ---

    #[test]
    fn is_footer_line_accepts_real_footer() {
        assert!(is_footer_line("[sha384:base62|lzma:uuencode]"));
    }

    #[test]
    fn is_footer_line_accepts_with_whitespace() {
        assert!(is_footer_line("  [sha384:base62|lzma:uuencode]  "));
    }

    #[test]
    fn is_footer_line_rejects_markdown_link() {
        assert!(!is_footer_line("[link|here]"));
    }

    #[test]
    fn is_footer_line_rejects_plain_text() {
        assert!(!is_footer_line("just some words"));
    }

    #[test]
    fn is_footer_line_rejects_empty() {
        assert!(!is_footer_line(""));
    }

    #[test]
    fn is_footer_line_rejects_unknown_compress_algo() {
        // W1: structural shape alone is not enough. A line that
        // satisfies `[a:b|c:d]` but where `c` is not a real
        // compression algorithm must be rejected.
        assert!(!is_footer_line("[sha384:base62|notarealalgo:uuencode]"));
        assert!(!is_footer_line("[anything:anything|anything:anything]"));
    }

    #[test]
    fn is_footer_line_accepts_each_known_algo() {
        // Spot-check that the vocabulary lift in commit.rs covers all
        // algorithms the encoder is allowed to choose. If a new algo
        // is added to the encoder without updating
        // `is_known_compress_algo`, this test fails.
        for algo in ["lzma", "zstd", "brotli", "gzip", "gz", "lz4", "snappy"] {
            let line = format!("[sha384:base62|{}:uuencode]", algo);
            assert!(
                is_footer_line(&line),
                "is_footer_line must accept known algo {}",
                algo
            );
        }
    }
}
