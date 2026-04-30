//! Codex archive subsystem.
//!
//! Splits across four files (was a single `archive.rs` until the codex
//! unification PR 2). Layout:
//!
//! - `mod.rs` — public entry points (`save_session`, `collect_archives`,
//!   `get_codex_dir`, `get_base_archive_name`) plus the forthcoming
//!   `ArchiveRequest` / `ArchiveResult` plumbing.
//! - `write.rs` — the per-session writer (`archive_session` body) and the
//!   `--all` driver loop.
//! - `sources.rs` — source walkers (today: `find_agent_sessions`; later:
//!   MCP / tool-output / history).
//! - `paths.rs` — archive-folder naming utilities (`determine_archive_dir`,
//!   `parse_archive_name`, `extract_short_id`, `get_base_archive_name`).
//!
//! This split is pure plumbing — every existing CLI invocation continues
//! to produce byte-identical output. Functional changes (new sidecars,
//! `--include` flag, by-project index) land in subsequent commits.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ArchiveEntry, Manifest};

mod paths;
mod sources;
mod write;

// Re-exports kept at the historical paths so `super::archive::*` callers
// (notably `migrate.rs` and `read.rs`) need no changes.
pub(super) use paths::{get_base_archive_name, parse_archive_name};
pub(super) use write::archive_session;

/// Archive the current session to the codex.
///
/// Public CLI entry point. Routes between single-session and bulk
/// (`--all`) modes; both paths produce byte-identical output to the
/// pre-split implementation.
pub(crate) fn save_session(
    session_path: Option<String>,
    all: bool,
    clean: bool,
    include_agents: bool,
) -> Result<()> {
    if all {
        write::save_all_sessions(clean, include_agents)?;
    } else {
        let path = resolve_session_path(session_path)?;
        archive_session(&path, clean, include_agents)?;
    }
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
