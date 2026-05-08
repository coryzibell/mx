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
            no_cleanup,
        } => {
            commit::pr_merge(number, rebase, merge_commit, no_cleanup)?;
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
    // mid-fix: `mx codex archive --backfill`. Every other handler emits
    // the warning at most once per process via the OnceLock guard
    // inside `warn_if_vault_present`.
    let suppress_vault_warning = matches!(
        cmd,
        CodexCommands::Archive {
            args: ArchiveArgs {
                backfill: Some(_),
                ..
            },
        } | CodexCommands::Save {
            args: ArchiveArgs {
                backfill: Some(_),
                ..
            },
        }
    );
    codex::notices::warn_if_vault_present(suppress_vault_warning);

    match cmd {
        // Deprecated alias: print a one-shot notice and fall through to
        // the canonical Archive handler.
        CodexCommands::Save { args } => {
            eprintln!("note: `mx codex save` is deprecated; use `mx codex archive` instead.");
            handle_codex_archive(args)
        }
        CodexCommands::Archive { args } => handle_codex_archive(args),
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

/// Shared implementation for `mx codex archive` (and its deprecated
/// `save` alias). Extracted so both CLI variants dispatch to the same
/// handler without duplicating the business logic.
fn handle_codex_archive(args: ArchiveArgs) -> Result<()> {
    let ArchiveArgs {
        path,
        all,
        clean,
        include_agents,
        include,
        backfill,
    } = args;

    let include_set = codex::IncludeSet::parse(&include)?;
    // W3: --include-agents only does anything when subagents are
    // also being captured. Silently doing nothing was confusing
    // -- fail-loud so the operator can correct the invocation.
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

    codex::archive_session(path, all, clean, include_agents, include_set)?;
    Ok(())
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

// ─── mx log: data structures ───────────────────────────────────────

/// A single commit harvested from `git log` structured output.
#[derive(Debug)]
struct ParsedCommit {
    full_hash: String,
    short_hash: String,
    decorations: String,
    parent_hashes: String,
    author: String,
    date: String,
    committer: String,
    commit_date: String,
    raw_subject: String,
    decoded: DecodedCommit,
    diff_block: Option<String>,
}

/// How to display the log output.
#[derive(Debug, Clone, PartialEq)]
enum LogDisplayMode {
    /// Default: `<short-hash> <decoded-subject>` (backward compat)
    Compact,
    /// `--full`: full header + decoded body (backward compat)
    Full,
    /// `--oneline`: `<short-hash> <decorations> <decoded-subject>`
    Oneline,
    /// `--format=short` / `--pretty=short`
    FormatShort,
    /// `--format=medium` / `--pretty=medium` (git's default)
    FormatMedium,
    /// `--format=full` / `--pretty=full`
    FormatFull,
    /// `--format=fuller` / `--pretty=fuller`
    FormatFuller,
    /// Custom format string -- passthrough to raw git
    CustomFormat(String),
}

/// Whether to attach diff output and in what form.
#[derive(Debug, Clone, PartialEq)]
enum DiffMode {
    None,
    Stat,
    ShortStat,
    Patch,
}

/// Parsed result of the user's CLI args for `mx log`.
#[derive(Debug)]
struct LogOptions {
    count: Option<usize>,
    display_mode: LogDisplayMode,
    diff_mode: DiffMode,
    decorate: Option<bool>,
    filter_args: Vec<String>,
}

/// Parse raw CLI args into structured `LogOptions`.
///
/// We intercept display/diff/count args and pass everything else
/// through as git filter args.
fn parse_log_args(args: Vec<String>) -> LogOptions {
    let mut count: Option<usize> = None;
    let mut display_mode = LogDisplayMode::Compact;
    let mut diff_mode = DiffMode::None;
    let mut decorate: Option<bool> = None;
    let mut filter_args: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        // ── count: -N shorthand ─────────────────────────────────
        // Must come before general flag checks. Matches `-3`, `-10`, etc.
        if arg.starts_with('-')
            && !arg.starts_with("--")
            && arg.len() > 1
            && arg[1..].chars().all(|c| c.is_ascii_digit())
        {
            count = arg[1..].parse().ok();
            i += 1;
            continue;
        }

        // ── count: -n N, -nN, --max-count=N ────────────────────
        if arg == "-n" {
            if i + 1 < args.len() {
                count = args[i + 1].parse().ok();
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if arg.starts_with("-n") && !arg.starts_with("--") && arg.len() > 2 {
            count = arg[2..].parse().ok();
            i += 1;
            continue;
        }
        if let Some(val) = arg.strip_prefix("--max-count=") {
            count = val.parse().ok();
            i += 1;
            continue;
        }

        // ── display mode: --oneline ────────────────────────────
        if arg == "--oneline" {
            display_mode = LogDisplayMode::Oneline;
            i += 1;
            continue;
        }

        // ── display mode: --full (mx-specific, backward compat) ─
        if arg == "--full" {
            display_mode = LogDisplayMode::Full;
            i += 1;
            continue;
        }

        // ── display mode: --format=<X> / --pretty=<X> ──────────
        if let Some(val) = arg
            .strip_prefix("--format=")
            .or_else(|| arg.strip_prefix("--pretty="))
        {
            display_mode = match val {
                "oneline" => LogDisplayMode::Oneline,
                "short" => LogDisplayMode::FormatShort,
                "medium" => LogDisplayMode::FormatMedium,
                "full" => LogDisplayMode::FormatFull,
                "fuller" => LogDisplayMode::FormatFuller,
                other => LogDisplayMode::CustomFormat(other.to_string()),
            };
            i += 1;
            continue;
        }
        // Bare --format / --pretty followed by a separate arg
        if arg == "--format" || arg == "--pretty" {
            if i + 1 < args.len() {
                let val = &args[i + 1];
                display_mode = match val.as_str() {
                    "oneline" => LogDisplayMode::Oneline,
                    "short" => LogDisplayMode::FormatShort,
                    "medium" => LogDisplayMode::FormatMedium,
                    "full" => LogDisplayMode::FormatFull,
                    "fuller" => LogDisplayMode::FormatFuller,
                    other => LogDisplayMode::CustomFormat(other.to_string()),
                };
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        // ── diff mode ──────────────────────────────────────────
        if arg == "--stat" {
            diff_mode = DiffMode::Stat;
            i += 1;
            continue;
        }
        if arg == "--shortstat" {
            diff_mode = DiffMode::ShortStat;
            i += 1;
            continue;
        }
        if arg == "-p" || arg == "--patch" {
            diff_mode = DiffMode::Patch;
            i += 1;
            continue;
        }

        // ── decorate ───────────────────────────────────────────
        if arg == "--decorate" {
            decorate = Some(true);
            i += 1;
            continue;
        }
        if arg == "--no-decorate" {
            decorate = Some(false);
            i += 1;
            continue;
        }

        // ── everything else: filter passthrough ────────────────
        filter_args.push(arg.clone());
        i += 1;
    }

    LogOptions {
        count,
        display_mode,
        diff_mode,
        decorate,
        filter_args,
    }
}

// ─── mx log: harvest ───────────────────────────────────────────────

/// Harvest commit data via a single structured `git log` call.
fn harvest_commits(opts: &LogOptions) -> Result<Vec<ParsedCommit>> {
    use std::process::Command;

    let harvest_format = "---MX-LOG---%n%H%n%h%n%D%n%p%n%an <%ae>%n%ad%n%cn <%ce>%n%cd%n%s%n---MX-BODY---%n%b%n---MX-LOG-END---";

    let mut cmd = Command::new("git");
    cmd.arg("log");

    let effective_count = opts.count.unwrap_or(10);
    cmd.arg(format!("-{}", effective_count));

    cmd.arg(format!("--format={}", harvest_format));

    // Pass through filter args
    for arg in &opts.filter_args {
        cmd.arg(arg);
    }

    cmd.stderr(std::process::Stdio::inherit());

    let output = cmd.output().context("Failed to run git log")?;

    if !output.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();

    for block in raw.split("---MX-LOG-END---") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        // Strip the leading sentinel
        let block = block.strip_prefix("---MX-LOG---").unwrap_or(block);
        let block = block.trim();

        // Split on body sentinel
        let (header_part, body_part) = if let Some(idx) = block.find("---MX-BODY---") {
            (&block[..idx], block[idx + "---MX-BODY---".len()..].trim())
        } else {
            (block, "")
        };

        let header_lines: Vec<&str> = header_part.lines().collect();
        if header_lines.len() < 9 {
            continue;
        }

        let full_hash = header_lines[0].to_string();
        let short_hash = header_lines[1].to_string();
        let decorations = header_lines[2].to_string();
        let parent_hashes = header_lines[3].to_string();
        let author = header_lines[4].to_string();
        let date = header_lines[5].to_string();
        let committer = header_lines[6].to_string();
        let commit_date = header_lines[7].to_string();
        let raw_subject = header_lines[8..].join("\n");

        let decoded = try_decode_commit_body(body_part);

        commits.push(ParsedCommit {
            full_hash,
            short_hash,
            decorations,
            parent_hashes,
            author,
            date,
            committer,
            commit_date,
            raw_subject,
            decoded,
            diff_block: None,
        });
    }

    Ok(commits)
}

// ─── mx log: diff attachment ───────────────────────────────────────

/// Attach diff blocks to commits by running a second git log pass.
fn attach_diffs(commits: &mut [ParsedCommit], opts: &LogOptions) -> Result<()> {
    use std::process::Command;

    if opts.diff_mode == DiffMode::None || commits.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("log");

    let effective_count = opts.count.unwrap_or(10);
    cmd.arg(format!("-{}", effective_count));

    cmd.arg("--format=---MX-DIFF--%H");

    match opts.diff_mode {
        DiffMode::Stat => {
            cmd.arg("--stat");
        }
        DiffMode::ShortStat => {
            cmd.arg("--shortstat");
        }
        DiffMode::Patch => {
            cmd.arg("-p");
        }
        DiffMode::None => unreachable!(),
    }

    for arg in &opts.filter_args {
        cmd.arg(arg);
    }

    cmd.stderr(std::process::Stdio::inherit());

    let output = cmd.output().context("Failed to run git log (diff pass)")?;

    if !output.status.success() {
        // Non-fatal: we still have the decoded headers
        return Ok(());
    }

    let raw = String::from_utf8_lossy(&output.stdout);

    // Build a map from hash -> diff content
    let mut diff_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for block in raw.split("---MX-DIFF--") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        // First line is the hash, rest is the diff
        let first_newline = block.find('\n');
        let hash = match first_newline {
            Some(idx) => block[..idx].trim().to_string(),
            None => block.trim().to_string(),
        };
        let diff_content = match first_newline {
            Some(idx) => block[idx + 1..].to_string(),
            None => String::new(),
        };
        if !hash.is_empty() {
            diff_map.insert(hash, diff_content);
        }
    }

    // Attach to commits
    for commit in commits.iter_mut() {
        if let Some(diff) = diff_map.remove(&commit.full_hash) {
            commit.diff_block = Some(diff);
        }
    }

    Ok(())
}

// ─── mx log: rendering ────────────────────────────────────────────

/// Format decoration string like git does: ` (HEAD -> main, origin/main)`
fn format_decorations(decorations: &str) -> String {
    let d = decorations.trim();
    if d.is_empty() {
        return String::new();
    }
    format!(" \x1b[33m(\x1b[0m{}\x1b[33m)\x1b[0m", d)
}

/// Get the display subject: decoded first line if available, else raw subject.
fn display_subject(commit: &ParsedCommit) -> String {
    if commit.decoded.was_decoded {
        // Use the first line of the decoded body as the subject
        commit
            .decoded
            .subject
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        // For non-decoded commits, the decoded.subject holds the passthrough
        // text which may be multi-line. Take only the first line as subject.
        commit
            .decoded
            .subject
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    }
}

/// Get the full display body: decoded body if available, else raw body
/// from non-decoded commits (plain git commits can have multi-line bodies).
fn display_body(commit: &ParsedCommit) -> Option<String> {
    // Lines beyond the first form the body, whether decoded or not.
    // For non-decoded commits, decoded.subject holds the passthrough
    // text (the full raw body from %b), which can be multi-line.
    let lines: Vec<&str> = commit.decoded.subject.lines().collect();
    if lines.len() > 1 {
        Some(lines[1..].join("\n"))
    } else {
        None
    }
}

/// Render commits in Compact mode (default).
fn render_compact(commits: &[ParsedCommit]) {
    for commit in commits {
        let subject = display_subject(commit);
        let truncated = safe_truncate(&subject, 72);
        println!("\x1b[33m{}\x1b[0m {}", commit.short_hash, truncated);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                println!("{}", diff_trimmed);
            }
        }
    }
}

/// Render commits in Full mode (--full, backward compat).
fn render_full(commits: &[ParsedCommit]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for commit in commits {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m", commit.full_hash);
        if commit.parent_hashes.split_whitespace().count() >= 2 {
            let _ = writeln!(out, "Merge:  {}", commit.parent_hashes);
        }
        let _ = writeln!(out, "Author: {}", commit.author);
        let _ = writeln!(out, "Date:   {}", commit.date);
        let _ = writeln!(out);

        // Print decoded message
        if commit.decoded.was_decoded {
            for line in commit.decoded.subject.lines() {
                let _ = writeln!(out, "    {}", line);
            }
            if let Some(trailing) = commit.decoded.trailing.as_deref() {
                let _ = writeln!(out);
                for line in trailing.lines() {
                    let _ = writeln!(out, "    \x1b[2m{}\x1b[0m", line);
                }
            }
        } else {
            let _ = writeln!(out, "    {}", commit.raw_subject);
        }
        let _ = writeln!(out);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                let _ = writeln!(out, "{}", diff_trimmed);
                let _ = writeln!(out);
            }
        }
    }
}

/// Render commits in Oneline mode.
fn render_oneline(commits: &[ParsedCommit], show_decorate: bool) {
    for commit in commits {
        let subject = display_subject(commit);
        let deco = if show_decorate {
            format_decorations(&commit.decorations)
        } else {
            String::new()
        };
        println!("\x1b[33m{}\x1b[0m{} {}", commit.short_hash, deco, subject);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                println!("{}", diff_trimmed);
            }
        }
    }
}

/// Render commits in git's `short` format.
fn render_format_short(commits: &[ParsedCommit]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for commit in commits {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m", commit.full_hash);
        let _ = writeln!(out, "Author: {}", commit.author);
        let _ = writeln!(out);
        let subject = display_subject(commit);
        let _ = writeln!(out, "    {}", subject);
        let _ = writeln!(out);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                let _ = writeln!(out, "{}", diff_trimmed);
                let _ = writeln!(out);
            }
        }
    }
}

/// Render commits in git's `medium` format (git's default).
fn render_format_medium(commits: &[ParsedCommit]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for commit in commits {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m", commit.full_hash);
        let _ = writeln!(out, "Author: {}", commit.author);
        let _ = writeln!(out, "Date:   {}", commit.date);
        let _ = writeln!(out);
        let subject = display_subject(commit);
        let _ = writeln!(out, "    {}", subject);
        if let Some(body) = display_body(commit) {
            let _ = writeln!(out);
            for line in body.lines() {
                let _ = writeln!(out, "    {}", line);
            }
        }
        let _ = writeln!(out);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                let _ = writeln!(out, "{}", diff_trimmed);
                let _ = writeln!(out);
            }
        }
    }
}

/// Render commits in git's `full` format.
fn render_format_full(commits: &[ParsedCommit]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for commit in commits {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m", commit.full_hash);
        if commit.parent_hashes.split_whitespace().count() >= 2 {
            let _ = writeln!(out, "Merge:  {}", commit.parent_hashes);
        }
        let _ = writeln!(out, "Author: {}", commit.author);
        // git's `full` format shows "Commit:" with the committer identity
        let _ = writeln!(out, "Commit: {}", commit.committer);
        let _ = writeln!(out);
        let subject = display_subject(commit);
        let _ = writeln!(out, "    {}", subject);
        if let Some(body) = display_body(commit) {
            let _ = writeln!(out);
            for line in body.lines() {
                let _ = writeln!(out, "    {}", line);
            }
        }
        let _ = writeln!(out);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                let _ = writeln!(out, "{}", diff_trimmed);
                let _ = writeln!(out);
            }
        }
    }
}

/// Render commits in git's `fuller` format.
fn render_format_fuller(commits: &[ParsedCommit]) {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for commit in commits {
        let _ = writeln!(out, "\x1b[33mcommit {}\x1b[0m", commit.full_hash);
        if commit.parent_hashes.split_whitespace().count() >= 2 {
            let _ = writeln!(out, "Merge:      {}", commit.parent_hashes);
        }
        let _ = writeln!(out, "Author:     {}", commit.author);
        let _ = writeln!(out, "AuthorDate: {}", commit.date);
        let _ = writeln!(out, "Commit:     {}", commit.committer);
        let _ = writeln!(out, "CommitDate: {}", commit.commit_date);
        let _ = writeln!(out);
        let subject = display_subject(commit);
        let _ = writeln!(out, "    {}", subject);
        if let Some(body) = display_body(commit) {
            let _ = writeln!(out);
            for line in body.lines() {
                let _ = writeln!(out, "    {}", line);
            }
        }
        let _ = writeln!(out);

        if let Some(ref diff) = commit.diff_block {
            let diff_trimmed = diff.trim();
            if !diff_trimmed.is_empty() {
                let _ = writeln!(out, "{}", diff_trimmed);
                let _ = writeln!(out);
            }
        }
    }
}

// ─── mx log: main handler ─────────────────────────────────────────

/// Handle `mx log` -- decoded git log with full git-log parity.
///
/// Four-phase architecture:
///   1. Parse args into LogOptions
///   2. Harvest commits via structured git log
///   3. Attach diffs (conditional)
///   4. Render via our own formatters (with decoded messages)
///
/// Custom --format / --graph: passthrough to raw git with a stderr note.
pub(crate) fn handle_log(args: Vec<String>) -> Result<()> {
    use std::process::Command;

    let opts = parse_log_args(args);

    // ── CustomFormat / --graph: passthrough ─────────────────────
    if let LogDisplayMode::CustomFormat(ref fmt) = opts.display_mode {
        eprintln!("note: custom --format bypasses message decoding");
        let mut cmd = Command::new("git");
        cmd.arg("log");
        let effective_count = opts.count.unwrap_or(10);
        cmd.arg(format!("-{}", effective_count));
        cmd.arg(format!("--format={}", fmt));
        match opts.diff_mode {
            DiffMode::Stat => {
                cmd.arg("--stat");
            }
            DiffMode::ShortStat => {
                cmd.arg("--shortstat");
            }
            DiffMode::Patch => {
                cmd.arg("-p");
            }
            DiffMode::None => {}
        }
        for arg in &opts.filter_args {
            cmd.arg(arg);
        }
        cmd.stderr(std::process::Stdio::inherit());
        let status = cmd.status().context("Failed to run git log")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    // Check if --graph was in filter_args (passthrough)
    if opts.filter_args.iter().any(|a| a == "--graph") {
        eprintln!("note: --graph bypasses message decoding");
        let mut cmd = Command::new("git");
        cmd.arg("log");
        let effective_count = opts.count.unwrap_or(10);
        cmd.arg(format!("-{}", effective_count));
        cmd.arg("--graph");
        match opts.display_mode {
            LogDisplayMode::Oneline => {
                cmd.arg("--oneline");
            }
            LogDisplayMode::Full => {
                // mx's `Full` mode is its own display (decoded body with
                // full header). When falling through to raw git for
                // --graph, map to git's default (medium), not git's
                // `full` which is a different thing.
                cmd.arg("--format=medium");
            }
            LogDisplayMode::FormatShort => {
                cmd.arg("--format=short");
            }
            LogDisplayMode::FormatMedium => {
                cmd.arg("--format=medium");
            }
            LogDisplayMode::FormatFull => {
                cmd.arg("--format=full");
            }
            LogDisplayMode::FormatFuller => {
                cmd.arg("--format=fuller");
            }
            _ => {}
        }
        match opts.diff_mode {
            DiffMode::Stat => {
                cmd.arg("--stat");
            }
            DiffMode::ShortStat => {
                cmd.arg("--shortstat");
            }
            DiffMode::Patch => {
                cmd.arg("-p");
            }
            DiffMode::None => {}
        }
        for arg in &opts.filter_args {
            if arg == "--graph" {
                continue; // already added
            }
            cmd.arg(arg);
        }
        cmd.stderr(std::process::Stdio::inherit());
        let status = cmd.status().context("Failed to run git log")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    // ── Phase 1-2: Harvest + Decode ────────────────────────────
    let mut commits = harvest_commits(&opts)?;

    // ── Phase 3: Attach diffs ──────────────────────────────────
    attach_diffs(&mut commits, &opts)?;

    // ── Phase 4: Render ────────────────────────────────────────
    let show_decorate = opts.decorate.unwrap_or(true);

    match opts.display_mode {
        LogDisplayMode::Compact => render_compact(&commits),
        LogDisplayMode::Full => render_full(&commits),
        LogDisplayMode::Oneline => render_oneline(&commits, show_decorate),
        LogDisplayMode::FormatShort => render_format_short(&commits),
        LogDisplayMode::FormatMedium => render_format_medium(&commits),
        LogDisplayMode::FormatFull => render_format_full(&commits),
        LogDisplayMode::FormatFuller => render_format_fuller(&commits),
        LogDisplayMode::CustomFormat(_) => unreachable!("handled above"),
    }

    Ok(())
}

/// Handle `mx show` -- decoded `git show`.
///
/// Two-pass approach:
///   Pass 1: `git show --format=<custom> --no-patch` to get commit metadata
///           + message. Decode the body using `try_decode_commit_body()`.
///   Pass 2: `git show --format="" <args>` to get the diff. Stream as-is.
///
/// Passthrough modes (skip decoding, run raw `git show`):
///   - Any arg matches the `ref:path` pattern (viewing file content).
///   - `--format` or `--pretty` present (user controls output format).
///
/// Fallback: if decoding fails, show the raw message (same as `git show`).
pub(crate) fn handle_show(args: Vec<String>) -> Result<()> {
    use std::io::Write;
    use std::process::Command;

    // ── Passthrough detection ───────────────────────────────────────
    let should_passthrough = args.iter().any(|a| {
        // --format or --pretty means user controls the format.
        if a == "--format" || a.starts_with("--format=") {
            return true;
        }
        if a == "--pretty" || a.starts_with("--pretty=") {
            return true;
        }
        // ref:path pattern -- viewing file content, not a commit.
        // Heuristic: contains ':' but isn't a flag value like --since=12:00.
        if !a.starts_with('-') && a.contains(':') {
            // Could be `HEAD:src/main.rs` or `abc123:README.md`.
            // Exclude things that look like times (digits:digits).
            let parts: Vec<&str> = a.splitn(2, ':').collect();
            if parts.len() == 2 {
                let after_colon = parts[1];
                // If after-colon is purely digits, it's probably a time,
                // not a path. Otherwise it's ref:path.
                if !after_colon.chars().all(|c| c.is_ascii_digit()) {
                    return true;
                }
            }
        }
        false
    });

    if should_passthrough {
        // Pure passthrough: run `git show` with all args, no decoding.
        let status = Command::new("git")
            .arg("show")
            .args(&args)
            .status()
            .context("Failed to run git show")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    // ── Detect --no-patch ───────────────────────────────────────────
    let has_no_patch = args.iter().any(|a| a == "--no-patch" || a == "-s");

    // ── Pass 1: commit metadata + encoded message ───────────────────
    // Format: full hash, parent hashes, author, date, subject, body.
    // %ad uses the user's configured date format, matching git show's behavior.
    let format_str = "%H%n%p%n%an <%ae>%n%ad%n%s%n%b%n---MX-SHOW-END---";
    let mut cmd1 = Command::new("git");
    cmd1.args(["show", &format!("--format={}", format_str), "--no-patch"])
        .stderr(std::process::Stdio::inherit());
    for arg in &args {
        // Skip diff-presentation flags for Pass 1 (metadata only).
        if arg == "--stat"
            || arg == "--shortstat"
            || arg == "--numstat"
            || arg == "--name-only"
            || arg == "--name-status"
            || arg.starts_with("--diff-filter")
        {
            continue;
        }
        cmd1.arg(arg);
    }

    let output1 = cmd1.output().context("Failed to run git show (pass 1)")?;
    if !output1.status.success() {
        // Might be a blob/tree/tag -- fallback to raw git show.
        let status = Command::new("git")
            .arg("show")
            .args(&args)
            .status()
            .context("Failed to run git show (fallback)")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    let raw1 = String::from_utf8_lossy(&output1.stdout);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // Parse and print each commit block (handles multiple refs).
    for commit_block in raw1.split("---MX-SHOW-END---") {
        let commit_block = commit_block.trim();
        if commit_block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = commit_block.lines().collect();
        if lines.len() < 5 {
            // Not enough lines for a commit -- might be tag preamble.
            // Print as-is.
            for line in &lines {
                writeln!(out, "{}", line)?;
            }
            continue;
        }

        let hash = lines[0];
        let parent_hashes = lines[1];
        let author = lines[2];
        let date = lines[3];
        let raw_subject = lines[4]; // one-way hash, not decodable
        let body: String = lines[5..].join("\n");

        // Decode the body.
        let result = try_decode_commit_body(&body);

        // Print header (matching git show's default format).
        writeln!(out, "\x1b[33mcommit {}\x1b[0m", hash)?;
        if parent_hashes.split_whitespace().count() >= 2 {
            writeln!(out, "Merge:  {}", parent_hashes)?;
        }
        writeln!(out, "Author: {}", author)?;
        writeln!(out, "Date:   {}", date)?;
        writeln!(out)?;

        // Print decoded message. The decoded body's first line is the
        // subject (since the encoded title is a non-decodable hash).
        if result.was_decoded {
            for line in result.subject.lines() {
                writeln!(out, "    {}", line)?;
            }
            if let Some(trailing) = result.trailing.as_deref() {
                writeln!(out)?;
                for line in trailing.lines() {
                    writeln!(out, "    \x1b[2m{}\x1b[0m", line)?;
                }
            }
        } else {
            // Not encoded -- show the original subject line from git,
            // then the body below it (matching git show's default).
            writeln!(out, "    {}", raw_subject)?;
            let body_trimmed = body.trim();
            if !body_trimmed.is_empty() {
                writeln!(out)?;
                for line in body_trimmed.lines() {
                    writeln!(out, "    {}", line)?;
                }
            }
        }
        writeln!(out)?;
    }

    // ── Pass 2: diff output ─────────────────────────────────────────
    if !has_no_patch {
        let mut cmd2 = Command::new("git");
        cmd2.args(["show", "--format="])
            .stderr(std::process::Stdio::inherit());
        for arg in &args {
            cmd2.arg(arg);
        }

        let output2 = cmd2.output().context("Failed to run git show (pass 2)")?;
        if output2.status.success() {
            out.write_all(&output2.stdout)?;
        }
        // If pass 2 fails (e.g. binary file), that's OK -- the header
        // was already printed. git show's own stderr will have the error.
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
#[derive(Debug)]
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
mod codex_archive_validation_tests {
    //! W3 from Verdictia's PR #268 review: `--include-agents` without
    //! `subagents` in `--include` was a silent no-op. The handler now
    //! errors at the seam between CLI parsing and the codex writer; we
    //! cover that edge here.
    use super::*;
    use crate::cli::CodexCommands;

    fn archive_cmd(include_agents: bool, include: &str) -> CodexCommands {
        CodexCommands::Archive {
            args: ArchiveArgs {
                path: None,
                all: false,
                clean: true,
                include_agents,
                include: include.to_string(),
                backfill: None,
            },
        }
    }

    #[test]
    fn include_agents_without_subagents_errors() {
        let result = handle_codex(archive_cmd(true, "none"));
        let err = result.expect_err("--include-agents + --include none must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("--include-agents") && msg.contains("subagents"),
            "error must explain the constraint, got: {msg}"
        );
    }

    #[test]
    fn include_agents_without_subagents_token_errors_even_with_others() {
        // `mcp` alone -- no subagents in the set -- also fails.
        let result = handle_codex(archive_cmd(true, "mcp,history"));
        assert!(
            result.is_err(),
            "--include-agents + --include without subagents must error"
        );
    }

    /// Regression guard: both `mx codex archive` and the deprecated
    /// `mx codex save` must route through the same `handle_codex_archive`
    /// helper. We verify by asserting that both produce the same error
    /// for the same invalid input -- if they diverge, the alias is broken.
    #[test]
    fn save_alias_routes_to_same_handler_as_archive() {
        let archive_result = handle_codex(CodexCommands::Archive {
            args: ArchiveArgs {
                path: None,
                all: false,
                clean: true,
                include_agents: true,
                include: "none".to_string(),
                backfill: None,
            },
        });
        let save_result = handle_codex(CodexCommands::Save {
            args: ArchiveArgs {
                path: None,
                all: false,
                clean: true,
                include_agents: true,
                include: "none".to_string(),
                backfill: None,
            },
        });

        let archive_err = archive_result
            .expect_err("archive with invalid args must error")
            .to_string();
        let save_err = save_result
            .expect_err("save with invalid args must error")
            .to_string();

        assert_eq!(
            archive_err, save_err,
            "save alias must produce the same error as archive: \
             archive={archive_err:?}, save={save_err:?}"
        );
    }
}

#[cfg(test)]
mod parse_log_args_tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn dash_n_shorthand() {
        let opts = parse_log_args(args(&["-3"]));
        assert_eq!(opts.count, Some(3));
        assert_eq!(opts.display_mode, LogDisplayMode::Compact);
        assert_eq!(opts.diff_mode, DiffMode::None);
    }

    #[test]
    fn dash_n_space_count() {
        let opts = parse_log_args(args(&["-n", "5"]));
        assert_eq!(opts.count, Some(5));
    }

    #[test]
    fn dash_n_joined_count() {
        let opts = parse_log_args(args(&["-n5"]));
        assert_eq!(opts.count, Some(5));
    }

    #[test]
    fn max_count_equals() {
        let opts = parse_log_args(args(&["--max-count=7"]));
        assert_eq!(opts.count, Some(7));
    }

    #[test]
    fn oneline_mode() {
        let opts = parse_log_args(args(&["--oneline"]));
        assert_eq!(opts.display_mode, LogDisplayMode::Oneline);
    }

    #[test]
    fn full_mode() {
        let opts = parse_log_args(args(&["--full"]));
        assert_eq!(opts.display_mode, LogDisplayMode::Full);
    }

    #[test]
    fn stat_diff_mode() {
        let opts = parse_log_args(args(&["--stat"]));
        assert_eq!(opts.diff_mode, DiffMode::Stat);
    }

    #[test]
    fn shortstat_diff_mode() {
        let opts = parse_log_args(args(&["--shortstat"]));
        assert_eq!(opts.diff_mode, DiffMode::ShortStat);
    }

    #[test]
    fn patch_diff_mode() {
        let opts = parse_log_args(args(&["-p"]));
        assert_eq!(opts.diff_mode, DiffMode::Patch);

        let opts2 = parse_log_args(args(&["--patch"]));
        assert_eq!(opts2.diff_mode, DiffMode::Patch);
    }

    #[test]
    fn format_short() {
        let opts = parse_log_args(args(&["--format=short"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatShort);
    }

    #[test]
    fn format_medium() {
        let opts = parse_log_args(args(&["--format=medium"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatMedium);
    }

    #[test]
    fn format_full_preset() {
        let opts = parse_log_args(args(&["--format=full"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatFull);
    }

    #[test]
    fn format_fuller() {
        let opts = parse_log_args(args(&["--format=fuller"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatFuller);
    }

    #[test]
    fn pretty_equals_format() {
        let opts = parse_log_args(args(&["--pretty=short"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatShort);
    }

    #[test]
    fn custom_format() {
        let opts = parse_log_args(args(&["--format=%H %s"]));
        assert_eq!(
            opts.display_mode,
            LogDisplayMode::CustomFormat("%H %s".to_string())
        );
    }

    #[test]
    fn mixed_args() {
        let opts = parse_log_args(args(&["--oneline", "-5", "--author=foo"]));
        assert_eq!(opts.display_mode, LogDisplayMode::Oneline);
        assert_eq!(opts.count, Some(5));
        assert!(opts.filter_args.contains(&"--author=foo".to_string()));
    }

    #[test]
    fn no_args_defaults() {
        let opts = parse_log_args(args(&[]));
        assert_eq!(opts.count, None);
        assert_eq!(opts.display_mode, LogDisplayMode::Compact);
        assert_eq!(opts.diff_mode, DiffMode::None);
        assert!(opts.filter_args.is_empty());
        assert!(opts.decorate.is_none());
    }

    #[test]
    fn filter_args_pass_through() {
        let opts = parse_log_args(args(&[
            "--since=2024-01-01",
            "--author=alice",
            "--all",
            "--",
            "src/main.rs",
        ]));
        assert_eq!(opts.filter_args.len(), 5);
        assert!(opts.filter_args.contains(&"--since=2024-01-01".to_string()));
        assert!(opts.filter_args.contains(&"--author=alice".to_string()));
        assert!(opts.filter_args.contains(&"--all".to_string()));
        // "--" is a pathspec separator and should pass through
        assert!(opts.filter_args.contains(&"--".to_string()));
        assert!(opts.filter_args.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn decorate_flags() {
        let opts1 = parse_log_args(args(&["--decorate"]));
        assert_eq!(opts1.decorate, Some(true));

        let opts2 = parse_log_args(args(&["--no-decorate"]));
        assert_eq!(opts2.decorate, Some(false));
    }

    #[test]
    fn double_digit_count() {
        let opts = parse_log_args(args(&["-10"]));
        assert_eq!(opts.count, Some(10));
    }

    #[test]
    fn format_oneline_maps_to_oneline_mode() {
        let opts = parse_log_args(args(&["--format=oneline"]));
        assert_eq!(opts.display_mode, LogDisplayMode::Oneline);
    }

    #[test]
    fn bare_format_with_separate_arg() {
        let opts = parse_log_args(args(&["--format", "medium"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatMedium);
    }

    #[test]
    fn bare_pretty_with_separate_arg() {
        let opts = parse_log_args(args(&["--pretty", "fuller"]));
        assert_eq!(opts.display_mode, LogDisplayMode::FormatFuller);
    }
}
