use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::SyncCommands;

/// Base path for sync scripts
fn scripts_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".matrix")
        .join("artifacts")
        .join("bin")
}

/// Default sync cache directory for a repo
fn default_sync_dir(repo: &str) -> PathBuf {
    let repo_slug = repo.replace('/', "-");
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".matrix")
        .join("cache")
        .join("sync")
        .join(repo_slug)
}

pub fn handle_sync(cmd: SyncCommands) -> Result<()> {
    match cmd {
        SyncCommands::Pull {
            repo,
            output,
            dry_run,
        } => sync_pull(&repo, output, dry_run),

        SyncCommands::Push {
            repo,
            input,
            dry_run,
        } => sync_push(&repo, input, dry_run),

        SyncCommands::Labels { repo, dry_run } => sync_labels(&repo, dry_run),

        SyncCommands::Issues { repo, dry_run } => sync_issues(&repo, dry_run),
    }
}

fn sync_pull(repo: &str, output: Option<String>, dry_run: bool) -> Result<()> {
    let script = scripts_dir().join("pull_github.py");
    let output_dir = output
        .map(PathBuf::from)
        .unwrap_or_else(|| default_sync_dir(repo));

    // Ensure output directory exists
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("Failed to create output directory: {:?}", output_dir))?;

    let mut args = vec![
        script.to_string_lossy().to_string(),
        repo.to_string(),
        output_dir.to_string_lossy().to_string(),
    ];

    if dry_run {
        args.push("--dry-run".to_string());
    }

    run_python(&args)
}

fn sync_push(repo: &str, input: Option<String>, dry_run: bool) -> Result<()> {
    let script = scripts_dir().join("sync_github.py");
    let input_dir = input
        .map(PathBuf::from)
        .unwrap_or_else(|| default_sync_dir(repo));

    let mut args = vec![
        script.to_string_lossy().to_string(),
        repo.to_string(),
        input_dir.to_string_lossy().to_string(),
    ];

    if dry_run {
        args.push("--dry-run".to_string());
    }

    run_python(&args)
}

fn sync_labels(repo: &str, dry_run: bool) -> Result<()> {
    let script = scripts_dir().join("sync_labels.py");

    let mut args = vec![script.to_string_lossy().to_string(), repo.to_string()];

    if dry_run {
        args.push("--dry-run".to_string());
    }

    run_python(&args)
}

fn sync_issues(repo: &str, dry_run: bool) -> Result<()> {
    let script = scripts_dir().join("sync_issues.py");

    let mut args = vec![script.to_string_lossy().to_string(), repo.to_string()];

    if dry_run {
        args.push("--dry-run".to_string());
    }

    run_python(&args)
}

fn run_python(args: &[String]) -> Result<()> {
    let status = Command::new("python")
        .args(args)
        .status()
        .with_context(|| format!("Failed to execute: python {}", args.join(" ")))?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "Command failed with exit code: {}",
            status.code().unwrap_or(-1)
        )
    }
}
