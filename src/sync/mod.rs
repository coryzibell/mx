//! GitHub sync module - pure Rust implementation
//!
//! Replaces Python scripts with native Rust for:
//! - Pull: GitHub → YAML
//! - Push: YAML → GitHub
//! - Labels: Sync identity labels
//! - Issues: Bidirectional sync

pub mod commands;
pub mod github;
pub mod merge;
pub mod wiki;
pub mod yaml;

use anyhow::Result;
use std::path::PathBuf;

use crate::SyncCommands;

/// Default sync cache directory for a repo.
/// Delegates to `crate::paths::sync_cache_dir`; kept for sub-module convenience.
pub fn default_sync_dir(repo: &str) -> PathBuf {
    crate::paths::sync_cache_dir(repo)
}

/// Artifacts directory.
/// Delegates to `crate::paths::artifacts_dir`; kept for sub-module convenience.
pub fn artifacts_dir() -> PathBuf {
    crate::paths::artifacts_dir()
}

pub fn handle_sync(cmd: SyncCommands) -> Result<()> {
    match cmd {
        SyncCommands::Pull {
            repo,
            output,
            dry_run,
        } => commands::pull::run(&repo, output, dry_run),

        SyncCommands::Push {
            repo,
            input,
            dry_run,
        } => commands::push::run(&repo, input, dry_run),

        SyncCommands::Labels { repo, dry_run } => commands::labels::run(&repo, dry_run),

        SyncCommands::Issues { repo, dry_run } => commands::issues::run(&repo, dry_run),
    }
}
