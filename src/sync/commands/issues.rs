//! Issues sync command - bidirectional issue sync
//!
//! Combines pull and push for a full bidirectional sync.

use anyhow::Result;

use super::{pull, push};

/// Run bidirectional issue sync
pub fn run(repo: &str, dry_run: bool) -> Result<()> {
    println!("=== Bidirectional Issue Sync ===");
    println!();

    // First pull to get remote changes
    println!("--- Pull (GitHub → Local) ---");
    pull::run(repo, None, dry_run)?;

    println!();
    println!("--- Push (Local → GitHub) ---");
    push::run(repo, None, dry_run)?;

    println!();
    println!("=== Sync Complete ===");

    Ok(())
}
