//! GitHub sync module - pure Rust implementation
//!
//! Replaces Python scripts with native Rust for:
//! - Pull: GitHub → YAML
//! - Push: YAML → GitHub
//! - Labels: Sync identity labels
//! - Issues: Bidirectional sync

pub mod github;
pub mod yaml;
pub mod merge;
pub mod commands;
pub mod wiki;

use anyhow::Result;
use std::path::PathBuf;

use crate::SyncCommands;

/// Default sync cache directory for a repo
pub fn default_sync_dir(repo: &str) -> PathBuf {
    let repo_slug = repo.replace('/', "-");
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".matrix")
        .join("cache")
        .join("sync")
        .join(repo_slug)
}

/// Matrix artifacts directory
pub fn artifacts_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".matrix")
        .join("artifacts")
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
