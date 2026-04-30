//! Codex archive subsystem.
//!
//! Splits across several files (was a single `archive.rs` until the
//! codex unification PR 2). Layout:
//!
//! - `mod.rs` — public entry points (`save_session`, `collect_archives`,
//!   `get_codex_dir`, `get_base_archive_name`), plus the
//!   `ArchiveRequest` / `ArchiveResult` plumbing and `archive::run`.
//! - `include.rs` — `IncludeSet`, the opt-in source selector parsed from
//!   the `--include` CLI flag.
//! - `write.rs` — the per-session writer (`archive_session` body) and the
//!   `--all` driver loop.
//! - `sources.rs` — source walkers (today: `find_agent_sessions`; later:
//!   MCP / tool-output / history).
//! - `paths.rs` — archive-folder naming utilities (`determine_archive_dir`,
//!   `parse_archive_name`, `extract_short_id`, `get_base_archive_name`).
//!
//! `archive::run` is the one canonical entry point. The historical
//! `save_session` is a thin wrapper that builds an `ArchiveRequest` from
//! CLI args and calls `run`. Status-quo invocations
//! (`mx codex archive` with no `--include`) produce byte-identical
//! output to the pre-PR-2 implementation.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ArchiveEntry, Manifest};

mod include;
mod paths;
mod sources;
mod write;

// Re-exports kept at the historical paths so `super::archive::*` callers
// (notably `migrate.rs` and `read.rs`) need no changes.
pub(crate) use include::IncludeSet;
pub(crate) use paths::{get_base_archive_name, parse_archive_name};

/// One archive request — either a single session by path, or the bulk
/// "archive everything not yet archived" mode.
#[derive(Debug, Clone)]
pub enum ArchiveRequest {
    /// Archive a specific session JSONL.
    Single(PathBuf),
    /// Walk `~/.claude/projects/` and archive every unarchived session.
    All,
}

/// Optional knobs that apply to every `ArchiveRequest`.
#[derive(Debug, Clone)]
pub struct ArchiveOptions {
    /// Clean mode: write `conversation.md` + images instead of the raw
    /// JSONL + agent files.
    pub clean: bool,
    /// Which optional source artifacts to capture.
    pub include: IncludeSet,
    /// Include sub-agent transcripts inside `conversation.md`. Only
    /// meaningful in clean mode; matches the historical
    /// `--include-agents` flag (still wired separately for backward
    /// compatibility — see `cli.rs`).
    pub include_agents_in_clean_md: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            clean: false,
            include: IncludeSet::status_quo(),
            include_agents_in_clean_md: false,
        }
    }
}

/// Outcome of a successful `archive::run`.
#[derive(Debug, Clone, Default)]
pub struct ArchiveResult {
    /// How many sessions were freshly archived (1 for `Single`; N for `All`).
    pub archived_count: usize,
    /// Sessions skipped because they were already archived (only meaningful
    /// for `ArchiveRequest::All`; always 0 for `Single` because the path
    /// is taken on faith — collisions are handled by suffix instead).
    pub skipped_count: usize,
    /// Resolved archive directory paths, in archive order. Useful for
    /// callers that want to chain follow-up work (e.g. printing, indexing).
    pub archive_paths: Vec<PathBuf>,
}

/// Canonical archive entry point. Builds the artifacts on disk according
/// to `request` and `options`, returns a summary.
///
/// Behavior with `IncludeSet::status_quo()` and `clean = false` is
/// byte-identical to the pre-PR-2 `mx codex archive` flow.
pub fn run(request: ArchiveRequest, options: ArchiveOptions) -> Result<ArchiveResult> {
    let mut result = ArchiveResult::default();

    match request {
        ArchiveRequest::Single(path) => {
            let archive_dir = write::archive_session(
                &path,
                options.clean,
                options.include_agents_in_clean_md,
                &options.include,
            )?;
            result.archived_count = 1;
            if let Some(dir) = archive_dir {
                result.archive_paths.push(dir);
            }
        }
        ArchiveRequest::All => {
            let summary = write::save_all_sessions(
                options.clean,
                options.include_agents_in_clean_md,
                &options.include,
            )?;
            result.archived_count = summary.archived_count;
            result.skipped_count = summary.skipped_count;
            result.archive_paths = summary.archive_paths;
        }
    }

    Ok(result)
}

/// Backwards-compatible CLI shim. Builds an `ArchiveRequest` from the
/// flat CLI args and delegates to `run`.
pub(crate) fn save_session(
    session_path: Option<String>,
    all: bool,
    clean: bool,
    include_agents: bool,
    include: IncludeSet,
) -> Result<()> {
    let request = if all {
        ArchiveRequest::All
    } else {
        let path = resolve_session_path(session_path)?;
        ArchiveRequest::Single(path)
    };
    let options = ArchiveOptions {
        clean,
        include,
        include_agents_in_clean_md: include_agents,
    };
    run(request, options)?;
    Ok(())
}

fn resolve_session_path(path: Option<String>) -> Result<PathBuf> {
    if let Some(p) = path {
        Ok(PathBuf::from(p))
    } else {
        crate::session::find_most_recent_session()
    }
}

/// Walk every archive dir under `codex_dir` and return one `ArchiveEntry`
/// per valid manifest. Used by `read.rs` (list/search) and `migrate.rs`.
pub(super) fn collect_archives(codex_dir: &Path) -> Result<Vec<ArchiveEntry>> {
    let mut archives = Vec::new();

    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }

        let manifest_content = fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = serde_json::from_str(&manifest_content)?;

        let dir_name = path.file_name().unwrap().to_string_lossy().to_string();
        let (short_id, incremental) = parse_archive_name(&dir_name);

        archives.push(ArchiveEntry {
            dir_name,
            short_id,
            incremental,
            manifest,
        });
    }

    Ok(archives)
}

pub(super) fn get_codex_dir() -> Result<PathBuf> {
    Ok(crate::paths::codex_dir())
}
