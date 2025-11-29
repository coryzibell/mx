//! Wiki sync infrastructure
//!
//! Git subprocess wrapper for wiki operations:
//! - Clone with token authentication
//! - Commit and push changes
//! - Page name sanitization
//! - Temporary directory management

use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::github::auth::get_github_token;

/// Clone a wiki repository to a target directory
pub fn clone_wiki(owner: &str, repo: &str, token: &str, target_dir: &Path) -> Result<()> {
    let wiki_url = format!(
        "https://x-access-token:{}@github.com/{}/{}.wiki.git",
        token, owner, repo
    );

    let output = Command::new("git")
        .args(["clone", &wiki_url, target_dir.to_str().unwrap()])
        .output()
        .context("Failed to execute git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Git clone failed: {}", stderr);
    }

    Ok(())
}

/// Run a git command in the specified directory
pub fn run_git(args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .context(format!("Failed to execute git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Git command failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Configure git user for commits
pub fn configure_git_user(wiki_dir: &Path, name: &str, email: &str) -> Result<()> {
    run_git(&["config", "user.name", name], wiki_dir)?;
    run_git(&["config", "user.email", email], wiki_dir)?;
    Ok(())
}

/// Commit all changes with a message
pub fn commit_changes(wiki_dir: &Path, message: &str) -> Result<()> {
    // Add all changes
    run_git(&["add", "."], wiki_dir)?;

    // Check if there are changes to commit
    let status = run_git(&["status", "--porcelain"], wiki_dir)?;
    if status.trim().is_empty() {
        return Ok(()); // No changes to commit
    }

    // Commit changes
    run_git(&["commit", "-m", message], wiki_dir)?;
    Ok(())
}

/// Push changes to remote
pub fn push_changes(wiki_dir: &Path) -> Result<()> {
    run_git(&["push", "origin", "master"], wiki_dir)?;
    Ok(())
}

/// Sanitize a page name for wiki compatibility
/// - Replace spaces with hyphens
/// - Remove non-alphanumeric chars (except hyphens)
/// - Lowercase
pub fn sanitize_page_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

/// Copy a file to the wiki directory with sanitized name
pub fn copy_to_wiki(source: &Path, wiki_dir: &Path, page_name: Option<&str>) -> Result<String> {
    let sanitized_name = if let Some(name) = page_name {
        sanitize_page_name(name)
    } else {
        // Use source filename without extension
        let stem = source
            .file_stem()
            .context("Invalid source file name")?
            .to_str()
            .context("Non-UTF8 filename")?;
        sanitize_page_name(stem)
    };

    // Ensure .md extension
    let target_name = if sanitized_name.ends_with(".md") {
        sanitized_name.clone()
    } else {
        format!("{}.md", sanitized_name)
    };

    let target_path = wiki_dir.join(&target_name);
    fs::copy(source, &target_path).context("Failed to copy file to wiki")?;

    Ok(target_name)
}

/// Check if a file should be skipped (numbered issue files)
fn should_skip_file(filename: &str) -> bool {
    lazy_static::lazy_static! {
        static ref NUMBERED_PATTERN: Regex = Regex::new(r"^\d+-").unwrap();
    }
    NUMBERED_PATTERN.is_match(filename)
}

/// Convert wiki page name to display name (remove .md, replace hyphens with spaces, title case)
fn display_page_name(page_name: &str) -> String {
    let name = page_name.trim_end_matches(".md");
    name.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Main sync command - sync file or directory to wiki
pub fn sync(repo: &str, source: &str, page_name: Option<&str>, dry_run: bool) -> Result<()> {
    // Parse repo
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repository format. Expected owner/repo");
    }
    let (owner, repo_name) = (parts[0], parts[1]);

    // Get source path
    let source_path = PathBuf::from(source);
    if !source_path.exists() {
        anyhow::bail!("Source path does not exist: {}", source);
    }

    // Validate page_name only valid for single file
    if page_name.is_some() && source_path.is_dir() {
        anyhow::bail!("--page-name can only be used with a single file");
    }

    println!("Syncing to {}/{} wiki...", owner, repo_name);
    if dry_run {
        println!("[DRY RUN MODE]");
    }

    // Get GitHub token
    let token = get_github_token().context("Failed to get GitHub token")?;

    // Create temp directory
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let wiki_dir = temp_dir.path();

    // Clone wiki
    println!("  Cloning wiki repository...");
    if !dry_run {
        clone_wiki(owner, repo_name, &token, wiki_dir)
            .context("Failed to clone wiki repository")?;
    }

    // Configure git user
    if !dry_run {
        configure_git_user(wiki_dir, "Matrix CLI", "noreply@github.com")?;
    }

    // Copy files
    println!("  Copying files...");
    let mut synced_pages = Vec::new();

    if source_path.is_file() {
        // Single file
        if !dry_run {
            let page = copy_to_wiki(&source_path, wiki_dir, page_name)?;
            synced_pages.push(page);
        }

        let display_name = if let Some(name) = page_name {
            name.to_string()
        } else {
            source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string()
        };
        println!("    {} → {}", source_path.display(), display_name);
    } else {
        // Directory - copy all .md files
        for entry in fs::read_dir(&source_path).context("Failed to read source directory")? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Skip non-markdown files
            if !filename.ends_with(".md") {
                continue;
            }

            // Skip numbered issue files
            if should_skip_file(filename) {
                println!("    (skipped: {})", filename);
                continue;
            }

            if !dry_run {
                let page = copy_to_wiki(&path, wiki_dir, None)?;
                synced_pages.push(page);
            }

            let display_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown");

            println!("    {} → {}", filename, display_page_name(display_name));
        }
    }

    if synced_pages.is_empty() {
        println!("\nNo files to sync.");
        return Ok(());
    }

    // Commit changes
    if !dry_run {
        println!("  Committing changes...");
        commit_changes(wiki_dir, "Sync from mx CLI")?;

        // Push to remote
        println!("  Pushing to remote...");
        push_changes(wiki_dir)?;
    }

    // Output wiki URL
    println!(
        "\nWiki synced: https://github.com/{}/{}/wiki",
        owner, repo_name
    );
    for page in &synced_pages {
        let page_url_name = page.trim_end_matches(".md");
        println!("  - {}", page_url_name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_page_name() {
        assert_eq!(sanitize_page_name("My Cool Page"), "my-cool-page");
        assert_eq!(sanitize_page_name("API Reference (v2)"), "api-reference-v2");
        assert_eq!(sanitize_page_name("Test@#$%Page"), "testpage");
        assert_eq!(sanitize_page_name("multi  spaces"), "multi--spaces");
    }

    #[test]
    fn test_should_skip_file() {
        assert!(should_skip_file("001-blocker-fix.md"));
        assert!(should_skip_file("25-issue-title.yaml"));
        assert!(should_skip_file("1-test.md"));
        assert!(!should_skip_file("README.md"));
        assert!(!should_skip_file("architecture.md"));
        assert!(!should_skip_file("test-001.md"));
    }

    #[test]
    fn test_display_page_name() {
        assert_eq!(display_page_name("my-cool-page"), "My-Cool-Page");
        assert_eq!(display_page_name("api-reference-v2"), "Api-Reference-V2");
        assert_eq!(display_page_name("readme.md"), "Readme");
    }
}
