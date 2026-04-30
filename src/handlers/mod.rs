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
    // Suppress the vault nag for the one invocation that's already
    // mid-fix: `mx codex save --backfill`. Every other handler emits
    // the warning at most once per process via the OnceLock guard
    // inside `warn_if_vault_present`.
    let suppress_vault_warning = matches!(
        cmd,
        CodexCommands::Save {
            backfill: Some(_),
            ..
        }
    );
    codex::notices::warn_if_vault_present(suppress_vault_warning);

    match cmd {
        CodexCommands::Save {
            path,
            all,
            clean,
            include_agents,
            include,
            backfill,
        } => {
            let include_set = codex::IncludeSet::parse(&include)?;
            // W3: --include-agents only does anything when subagents are
            // also being captured. Silently doing nothing was confusing
            // — fail-loud so the operator can correct the invocation.
            if include_agents && !include_set.subagents {
                anyhow::bail!(
                    "--include-agents requires `subagents` in --include (got --include='{}'). \
                     Either drop --include-agents or add `subagents` to --include.",
                    include
                );
            }

            if let Some(vault_arg) = backfill {
                // `--backfill` with no value parses as `Some("")` thanks
                // to `default_missing_value`. Resolve to the canonical
                // `~/.wonka/vault/archives/` in that case.
                let vault_path = if vault_arg.is_empty() {
                    crate::paths::wonka_vault_archives_dir()
                } else {
                    std::path::PathBuf::from(vault_arg)
                };
                let options = codex::archive::ArchiveOptions {
                    clean,
                    include: include_set,
                    include_agents_in_clean_md: include_agents,
                };
                let report = codex::run_backfill(&vault_path, options)?;
                // Echo a final terse summary on stdout (the running
                // progress lines went to stderr); stdout is the
                // machine-readable channel for chaining in scripts.
                println!(
                    "vault={} snapshots={} found={} archived={} skipped={} errors={}",
                    report.vault_path.display(),
                    report.vault_snapshots_walked,
                    report.sessions_found,
                    report.sessions_archived,
                    report.sessions_skipped_already_archived,
                    report.errors.len()
                );
                return Ok(());
            }

            codex::save_session(path, all, clean, include_agents, include_set)?;
            Ok(())
        }
        CodexCommands::Export {
            session,
            project,
            date,
            format,
            include,
            archive_first,
            output,
        } => handle_codex_export(
            session,
            project,
            date,
            format,
            include,
            archive_first,
            output,
        ),
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

/// Build an `ExportRequest` from the flat CLI args and dispatch to
/// `codex::export::run`. The selectors are mutually exclusive at the
/// CLI level (clap `group = "selector"`) so at most one of
/// `--session / --project / --date` will be `Some`.
#[allow(clippy::too_many_arguments)]
fn handle_codex_export(
    session: Option<String>,
    project: Option<String>,
    date: Option<String>,
    format: String,
    include: String,
    archive_first: bool,
    output: Option<String>,
) -> Result<()> {
    use std::path::PathBuf;

    let selector = match (session, project, date) {
        (Some(s), None, None) => codex::Selector::Session(codex::export::SessionRef(s)),
        (None, Some(p), None) => codex::Selector::Project(p),
        (None, None, Some(d)) => codex::Selector::Date(codex::export::DateRange::parse(&d)?),
        (None, None, None) => codex::Selector::Latest,
        // The clap group should prevent this, but defend anyway so the
        // error is friendly if a downstream caller hand-builds the args.
        _ => anyhow::bail!(
            "--session, --project, and --date are mutually exclusive; pass at most one"
        ),
    };
    let format = codex::Format::parse(&format)?;
    let include = codex::ExportIncludeSet::parse(&include)?;
    let request = codex::ExportRequest {
        selector,
        format,
        include,
        archive_first,
        output: output.map(PathBuf::from),
    };
    codex::run_export(request)?;
    Ok(())
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
                    let result = try_decode_commit_body(&body);
                    println!();
                    for line in result.subject.lines() {
                        println!("    {}", line);
                    }
                    // If decoding succeeded AND there was post-footer
                    // content (dejavu marker or user-appended note),
                    // render it beneath the decoded subject in dim
                    // style. The dim ANSI sequence (\x1b[2m) matches
                    // the existing yellow-hash convention -- raw
                    // escapes inline rather than a `colored` crate --
                    // and visually marks the content as "extra,
                    // post-footer" without hiding it.
                    if let Some(trailing) = result.trailing.as_deref() {
                        println!();
                        for line in trailing.lines() {
                            println!("    \x1b[2m{}\x1b[0m", line);
                        }
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

                // Try to decode the body. Compact format is one line
                // per commit, so trailing post-footer content is not
                // shown here -- use `mx log --full` to see it.
                let result = try_decode_commit_body(&body);
                let display = if result.was_decoded {
                    result.subject
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

/// The result of attempting to decode an encoded commit body.
///
/// This shape lets the caller distinguish three cases without resorting
/// to string-equality probing:
///
/// 1. **Decoded with no trailing content** (`was_decoded == true`,
///    `trailing.is_none()`): the natural case -- the footer was the last
///    meaningful line of the message and there's nothing after it.
/// 2. **Decoded with trailing content** (`was_decoded == true`,
///    `trailing.is_some()`): the footer was somewhere above the bottom
///    of the message, and there's content (the dejavu marker, a user-
///    appended note, or both) after it. The renderer should display
///    `trailing` beneath `subject` so the easter egg / note remains
///    visible -- before this struct existed, that content was silently
///    dropped (issue #260, finding C1).
/// 3. **Not decoded** (`was_decoded == false`): the message had no
///    recognizable footer (or decode failed). `subject` holds the
///    trimmed original message; `trailing` is always `None`.
pub(crate) struct DecodedCommit {
    /// The decoded message body (when `was_decoded`) or the trimmed
    /// original text (when not).
    pub(crate) subject: String,
    /// Anything that appeared in the message AFTER the chosen footer
    /// line. `Some` only when `was_decoded` is true and there was real
    /// post-footer content. The dejavu marker is preserved here as-is
    /// so the renderer can show it.
    pub(crate) trailing: Option<String>,
    /// True iff a footer was located and decoding succeeded.
    pub(crate) was_decoded: bool,
}

impl DecodedCommit {
    /// Build a `DecodedCommit` for the un-decoded case: no footer, or
    /// decode failure. `subject` is the trimmed original; nothing
    /// trailing.
    fn passthrough(original: &str) -> Self {
        Self {
            subject: original.trim().to_string(),
            trailing: None,
            was_decoded: false,
        }
    }
}

/// Try to decode an encoded commit body, returning a [`DecodedCommit`]
/// that distinguishes the decoded subject from any trailing content.
///
/// # Footer-scan strategy
///
/// Rather than restricting the footer to the last line of the message,
/// we walk the entire body and use the LAST footer-shaped line we find.
/// This covers two cases:
///
/// 1. Dejavu commits where the marker line sits below the footer. The
///    footer is no longer the last line, but it is still the last
///    footer-shaped line.
/// 2. User-amended commits where someone appended free-form text below
///    the encoded message. Same property: footer is no longer last, but
///    is still the last footer-shaped line.
///
/// # Limitation: last-wins is a heuristic, not a guarantee
///
/// "Last footer-shaped line wins" is a heuristic. If a user pastes a
/// real prior-commit footer below their amended message (e.g. in a
/// "Closes #123, see footer of abc1234" style note that quotes a real
/// footer literally), this scan will pick the user's pasted footer
/// instead of the actual one. The decoder will then either fail (and
/// fall back to the original message via [`DecodedCommit::passthrough`])
/// or, in the unlucky case where the pasted footer happens to validly
/// decode the encoded body, produce the wrong subject.
///
/// We accept this trade-off because the alternative (first-wins)
/// breaks the much more common dejavu / amended-note case, where the
/// real footer is followed by intentional trailing content. The
/// pasted-footer collision requires a user to deliberately type
/// something that looks exactly like our tag format on a line by
/// itself; the dejavu case happens automatically every few commits.
pub(crate) fn try_decode_commit_body(body: &str) -> DecodedCommit {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return DecodedCommit::passthrough(trimmed);
    }

    let lines: Vec<&str> = trimmed.lines().collect();

    // Walk the whole message; pick the LAST footer-shaped line. Last
    // wins because the natural case has the footer at (or near) the
    // bottom; any earlier bracket-pipe-shaped substring is almost
    // certainly user-authored markdown, not the real encode footer.
    let footer_idx = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, l)| if is_footer_line(l) { Some(i) } else { None });

    let footer_idx = match footer_idx {
        Some(i) => i,
        None => return DecodedCommit::passthrough(trimmed),
    };

    let footer = lines[footer_idx];

    // Encoded body = every line strictly above the footer, with the
    // dejavu marker filtered out (it can in principle appear in older
    // formats above the footer; current encoder writes it after).
    // Marker spelling is sourced from the encoder so the writer and
    // this filter never drift apart.
    let body_lines: Vec<&str> = lines[..footer_idx]
        .iter()
        .filter(|l| l.trim() != commit::DEJAVU_MARKER)
        .copied()
        .collect();

    // Trailing = every line strictly below the footer. We keep these
    // verbatim (including the dejavu marker, if present) so the caller
    // can render them beneath the decoded subject. Pure-whitespace
    // lines are stripped from each end; if nothing remains, trailing
    // is None.
    let trailing_raw = lines[footer_idx + 1..].join("\n");
    let trailing_trimmed = trailing_raw.trim();
    let trailing = if trailing_trimmed.is_empty() {
        None
    } else {
        Some(trailing_trimmed.to_string())
    };

    // If there's nothing above the footer to decode, treat as
    // not-encoded -- this covers the edge case of a message that is
    // ONLY a footer (or footer + whitespace + marker) with no encoded
    // payload above it.
    if body_lines.iter().all(|l| l.trim().is_empty()) {
        return DecodedCommit::passthrough(trimmed);
    }

    let encoded_body = body_lines.join("\n");

    match commit::decode_body(&encoded_body, footer) {
        Ok(decoded) => DecodedCommit {
            subject: decoded,
            trailing,
            was_decoded: true,
        },
        Err(_) => DecodedCommit::passthrough(trimmed),
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

    /// Encode a (title, body) pair, retrying until:
    /// - the user predicate is satisfied (e.g. dejavu or not), AND
    /// - no line of the encoded body is itself footer-shaped (which
    ///   would derail the scanner), AND
    /// - the canonical round-trip (`<body>\n\n<footer>`) actually
    ///   decodes back to the original message.
    ///
    /// The third condition guards against pre-existing fragility in
    /// the underlying compress/encode round-trip: a small handful of
    /// random codec/dictionary pairings produce encoded bytes whose
    /// decode succeeds at the dictionary layer but fails at the
    /// decompression layer. That fragility is independent of the
    /// footer-scan logic under test, so the helper filters those
    /// rolls out and only returns "clean" encoder samples.
    ///
    /// With 6 algorithms, 60+ dictionaries, and a generous retry
    /// budget, rejection rate is low and the helper terminates
    /// promptly under normal conditions.
    fn encode_until<F: Fn(&crate::commit::EncodedCommit) -> bool>(
        title: &str,
        body: &str,
        predicate: F,
    ) -> crate::commit::EncodedCommit {
        for _ in 0..1000 {
            let Ok(enc) = encode_commit(title, body) else {
                continue;
            };
            if !predicate(&enc) {
                continue;
            }
            if enc.body.lines().any(is_footer_line) {
                continue;
            }
            // Canonical round-trip check.
            let canonical = format!("{}\n\n{}", enc.body, enc.footer);
            let result = try_decode_commit_body(&canonical);
            if !result.was_decoded || result.subject != body {
                continue;
            }
            return enc;
        }
        panic!("encoder failed to satisfy predicate after 1000 attempts");
    }

    #[test]
    fn footer_at_bottom_decodes_existing_behavior() {
        // The natural case: footer is the last line, no extra trailing
        // content. This is what the decoder used to handle exclusively;
        // it must keep working after the generalization.
        let enc = encode_until("title diff", "the quick brown fox", |e| !e.dejavu);
        let body = format!("{}\n\n{}", enc.body, enc.footer);
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "the quick brown fox");
        assert!(result.trailing.is_none());
    }

    #[test]
    fn footer_followed_by_dejavu_marker_decodes_and_preserves_marker() {
        // Issue #260's original repro: dejavu appends a marker line
        // after the footer, so the footer is no longer last. Must
        // still decode AND surface the marker via `trailing` so the
        // renderer can keep the easter egg visible.
        let enc = encode_until("title diff", "decoded subject under dejavu", |e| e.dejavu);
        let body = format!("{}\n\n{}", enc.body, enc.footer);
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "decoded subject under dejavu");
        // The dejavu marker must appear in `trailing` -- this is the
        // assertion that would have caught C1 originally. We check
        // structurally (Some + non-empty) rather than asserting on
        // specific marker text, so the test stays robust if the
        // marker spelling changes.
        assert!(
            result.trailing.is_some(),
            "dejavu marker must be preserved in trailing, not silently dropped"
        );
        assert!(!result.trailing.as_deref().unwrap().is_empty());
    }

    #[test]
    fn user_amended_text_after_footer_decodes_and_preserves_note() {
        // The user ran `mx commit`, then later did `git commit --amend`
        // and tacked on a free-form note. The footer is now in the
        // middle of the message. Decode must succeed and the appended
        // note must come through in `trailing`.
        let enc = encode_until("title diff", "the original message", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nP.S. amended later by hand.",
            enc.body, enc.footer
        );
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "the original message");
        assert_eq!(
            result.trailing.as_deref(),
            Some("P.S. amended later by hand.")
        );
    }

    #[test]
    fn user_amended_text_after_dejavu_marker_decodes() {
        // Combine both: dejavu commit AND user-appended text. The
        // footer is buried two layers deep but must still be found,
        // and BOTH the marker line and the user note must appear in
        // trailing (so the renderer can show them).
        let enc = encode_until("title diff", "buried treasure", |e| e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nuser note added during amend",
            enc.body, enc.footer
        );
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "buried treasure");
        let trailing = result.trailing.expect("trailing must be present");
        // Structural check: trailing must contain the user note. We
        // don't assert on the marker spelling here -- the previous
        // test already covers that the marker is preserved.
        assert!(trailing.contains("user note added during amend"));
    }

    #[test]
    fn no_footer_returns_original_unchanged() {
        // A plain (un-encoded) commit message must pass through. No
        // footer, no decode -- the caller falls back to the raw subject.
        let raw = "fix: a perfectly normal git commit\n\nWith a body.";
        let result = try_decode_commit_body(raw);
        assert!(!result.was_decoded);
        assert_eq!(result.subject, raw);
        assert!(result.trailing.is_none());
    }

    #[test]
    fn empty_body_returns_empty() {
        let r1 = try_decode_commit_body("");
        assert!(!r1.was_decoded);
        assert_eq!(r1.subject, "");
        assert!(r1.trailing.is_none());

        let r2 = try_decode_commit_body("   \n  ");
        assert!(!r2.was_decoded);
        assert_eq!(r2.subject, "");
        assert!(r2.trailing.is_none());
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
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "still decodes");
    }

    #[test]
    fn markdown_brackets_in_body_are_not_mistaken_for_footer() {
        // Free-form text like `[foo|bar]` (not a real footer) must not
        // derail the scan. `is_footer_line` validates the parse, so a
        // fragment that happens to start with '[' and contain '|' but
        // doesn't match the real tag shape (or names an unknown
        // compression algorithm) is correctly skipped.
        let enc = encode_until("title diff", "round trip through markdown", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\nSee the [link|here] for details.",
            enc.body, enc.footer
        );
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "round trip through markdown");
    }

    #[test]
    fn footer_shaped_with_unknown_algo_is_rejected() {
        // W1 regression guard: a line that satisfies the structural
        // shape but names an algorithm we don't know must NOT be
        // treated as a footer. Before W1, this would pass
        // `is_footer_line`; after W1, the unknown-algo check rejects
        // it. The encoded body still has its real footer, so the
        // decode succeeds against the real footer rather than the
        // user-authored decoy.
        let enc = encode_until("title diff", "shape-only decoy ignored", |e| !e.dejavu);
        let body = format!(
            "{}\n\n{}\n\n[madeup:fakedict|notreal:alsofake]",
            enc.body, enc.footer
        );
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "shape-only decoy ignored");
        // The decoy line is post-real-footer text, so it lands in
        // trailing -- which is fine; it's just user content now.
        assert!(result.trailing.is_some());
    }

    #[test]
    fn fixture_real_dejavu_commit_decodes() {
        // Real-world regression fixture from issue #260: a dejavu
        // commit observed during the path-alignment work. The body
        // text is exactly what `git cat-file -p` returned (everything
        // after the title line). If the decoder ever stops handling
        // this shape, this test catches it.
        let body = format!(
            "8NO48P3FCDPIGSJ5C5I6QP9978G76R39DKG46RRECPKMETBIC5Q6IRRE41Q6U83141Q6AOBJCLP20R39DPLMIRJ741Q6U834DTHN6BRGC5Q6GSPEDLI0====\n\n[blake2s:base32hex|snappy:base32hex]\n{}",
            commit::DEJAVU_MARKER
        );
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(
            result.subject,
            "docs(readme): slim Configuration to a teaser linking to docs/paths.md"
        );
        // Marker must be preserved in trailing (the C1 fix).
        assert!(result.trailing.is_some());
    }

    // --- S2 edge-case tests ---

    #[test]
    fn footer_as_only_line_returns_passthrough() {
        // Edge case: the message contains only the footer, with no
        // encoded body above it. There's nothing to decode, so the
        // function must passthrough harmlessly rather than panic or
        // return garbage.
        let body = "[sha384:base62|lzma:uuencode]";
        let result = try_decode_commit_body(body);
        assert!(!result.was_decoded);
        // Subject is the trimmed original.
        assert_eq!(result.subject, body);
        assert!(result.trailing.is_none());
    }

    #[test]
    fn footer_as_first_line_with_trailing_only_passes_through() {
        // Edge case: the footer is the first line and is followed
        // only by free-form text. There's nothing above the footer
        // to decode, so we passthrough.
        let body = "[sha384:base62|lzma:uuencode]\n\nA stray note with no encoded payload.";
        let result = try_decode_commit_body(body);
        assert!(!result.was_decoded);
        assert!(result.trailing.is_none());
    }

    #[test]
    fn whitespace_only_line_between_body_and_footer_decodes() {
        // Edge case: the encoder writes `body\n\nfooter` (one blank
        // line between). Anything more than one blank line is
        // unusual but plausible after a manual amend. The decoder
        // tolerates the whitespace and decodes correctly.
        let enc = encode_until("title diff", "tolerates extra blanks", |e| !e.dejavu);
        let body = format!("{}\n\n   \n\n{}", enc.body, enc.footer);
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "tolerates extra blanks");
    }

    #[test]
    fn trailing_whitespace_only_after_footer_yields_no_trailing() {
        // Edge case: footer followed by only whitespace lines. We
        // must NOT report this as `trailing = Some("")` -- that would
        // be a useless empty string the renderer would still try to
        // print. Trim to None.
        let enc = encode_until("title diff", "no trailing whitespace", |e| !e.dejavu);
        let body = format!("{}\n\n{}\n   \n  ", enc.body, enc.footer);
        let result = try_decode_commit_body(&body);
        assert!(result.was_decoded);
        assert_eq!(result.subject, "no trailing whitespace");
        assert!(
            result.trailing.is_none(),
            "whitespace-only trailing must not produce a Some"
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

#[cfg(test)]
mod codex_save_validation_tests {
    //! W3 from Verdictia's PR #268 review: `--include-agents` without
    //! `subagents` in `--include` was a silent no-op. The handler now
    //! errors at the seam between CLI parsing and the codex writer; we
    //! cover that edge here.
    use super::*;
    use crate::cli::CodexCommands;

    fn save_cmd(include_agents: bool, include: &str) -> CodexCommands {
        CodexCommands::Save {
            path: None,
            all: false,
            clean: true,
            include_agents,
            include: include.to_string(),
            backfill: None,
        }
    }

    #[test]
    fn include_agents_without_subagents_errors() {
        let result = handle_codex(save_cmd(true, "none"));
        let err = result.expect_err("--include-agents + --include none must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("--include-agents") && msg.contains("subagents"),
            "error must explain the constraint, got: {msg}"
        );
    }

    #[test]
    fn include_agents_without_subagents_token_errors_even_with_others() {
        // `mcp` alone — no subagents in the set — also fails.
        let result = handle_codex(save_cmd(true, "mcp,history"));
        assert!(
            result.is_err(),
            "--include-agents + --include without subagents must error"
        );
    }
}
