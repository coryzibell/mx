//! Encoded commit functionality - the upload pattern
//!
//! Commits are encoded for maximum entropy:
//! - Title: Hash of diff, encoded with random dictionary
//! - Body: Message compressed and encoded with random dictionary
//! - Footer: Compression algorithm hint

use anyhow::{bail, Context, Result};
use rand::prelude::IndexedRandom;
use std::io::Write;
use std::process::{Command, Stdio};

const HASH_ALGOS: &[&str] = &["md5", "sha256", "sha512", "blake3", "xxh64", "xxh3"];
const COMPRESS_ALGOS: &[&str] = &["gzip", "zstd", "brotli", "lz4"];

/// Get the staged diff from git
pub fn get_staged_diff() -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "--staged"])
        .output()
        .context("Failed to run git diff")?;

    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Check if there are staged changes
pub fn has_staged_changes() -> Result<bool> {
    let diff = get_staged_diff()?;
    Ok(!diff.trim().is_empty())
}

/// Stage all changes
pub fn stage_all() -> Result<()> {
    let output = Command::new("git")
        .args(["add", "-A"])
        .output()
        .context("Failed to run git add")?;

    if !output.status.success() {
        bail!(
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Find base-d binary
fn find_base_d() -> Result<String> {
    // Check common locations
    let candidates = [
        "base-d",
        &format!(
            "{}/.cargo/bin/base-d",
            std::env::var("HOME").unwrap_or_default()
        ),
        &format!(
            "{}/.local/bin/base-d",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];

    for candidate in candidates {
        if Command::new("which")
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }

        // Also check if it exists directly
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.to_string());
        }
    }

    // Try to find via mise
    let mise_path = format!(
        "{}/.local/share/mise/installs/cargo-base-d",
        std::env::var("HOME").unwrap_or_default()
    );
    if let Ok(entries) = std::fs::read_dir(&mise_path) {
        for entry in entries.flatten() {
            let bin_path = entry.path().join("bin/base-d");
            if bin_path.exists() {
                return Ok(bin_path.to_string_lossy().to_string());
            }
        }
    }

    bail!("base-d not found. Install with: cargo install base-d")
}

/// Encode text using base-d with hash and random dictionary, returns (encoded, dict_name)
pub fn encode_hash(text: &str) -> Result<(String, String)> {
    let base_d = find_base_d()?;
    let algo = HASH_ALGOS.choose(&mut rand::rng()).unwrap();

    let mut child = Command::new(&base_d)
        .args(["--hash", algo, "--dejavu"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn base-d")?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .context("Failed to write to base-d stdin")?;

    let output = child
        .wait_with_output()
        .context("Failed to wait for base-d")?;

    // dejavu prints "Using dictionary: <name>" to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    let dict = stderr
        .lines()
        .find(|l| l.contains("Using dictionary:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if !output.status.success() {
        bail!("base-d hash failed: {}", stderr);
    }

    Ok((
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        dict,
    ))
}

/// Compress and encode text using base-d, returns (encoded, compress_algo, dict_name)
pub fn encode_compress(text: &str) -> Result<(String, String, String)> {
    let base_d = find_base_d()?;
    let algo = COMPRESS_ALGOS.choose(&mut rand::rng()).unwrap();

    let mut child = Command::new(&base_d)
        .args(["--compress", algo, "--dejavu"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn base-d")?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .context("Failed to write to base-d stdin")?;

    let output = child
        .wait_with_output()
        .context("Failed to wait for base-d")?;

    // dejavu prints "Using dictionary: <name>" to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    let dict = stderr
        .lines()
        .find(|l| l.contains("Using dictionary:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if !output.status.success() {
        bail!("base-d compress failed: {}", stderr);
    }

    let encoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((encoded, algo.to_string(), dict))
}

/// Create a git commit with the given message
pub fn git_commit(title: &str, body: &str, footer: &str) -> Result<()> {
    let message = format!("{}\n\n{}\n\n{}", title, body, footer);

    let output = Command::new("git")
        .args(["commit", "-m", &message])
        .output()
        .context("Failed to run git commit")?;

    if !output.status.success() {
        bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Pull with rebase to sync with remote (CI often pushes version bumps)
fn git_pull_rebase() -> Result<()> {
    let output = Command::new("git")
        .args(["pull", "--rebase"])
        .output()
        .context("Failed to run git pull --rebase")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore "no tracking branch" errors - just means nothing to pull
        if !stderr.contains("There is no tracking information")
            && !stderr.contains("no tracking information") {
            bail!("git pull --rebase failed: {}", stderr);
        }
    }

    Ok(())
}

/// Push to origin
pub fn git_push() -> Result<()> {
    // Always pull --rebase first to handle CI version bumps
    git_pull_rebase()?;

    let output = Command::new("git")
        .arg("push")
        .output()
        .context("Failed to run git push")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check if we need to set upstream
        if stderr.contains("no upstream branch") {
            let branch = get_current_branch()?;
            let output = Command::new("git")
                .args(["push", "-u", "origin", &branch])
                .output()
                .context("Failed to run git push -u")?;

            if !output.status.success() {
                bail!(
                    "git push failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        } else {
            bail!("git push failed: {}", stderr);
        }
    }

    Ok(())
}

/// Get current branch name
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to get current branch")?;

    if !output.status.success() {
        bail!(
            "Failed to get branch: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Perform the full upload commit
pub fn upload_commit(message: &str, stage_all_flag: bool, push: bool) -> Result<()> {
    // Stage if requested
    if stage_all_flag {
        stage_all()?;
    }

    // Check for staged changes
    if !has_staged_changes()? {
        bail!("No staged changes to commit");
    }

    // Get diff for hashing
    let diff = get_staged_diff()?;

    // Generate title (hash of diff) - random dictionary
    let (title, title_dict) = encode_hash(&diff)?;

    // Generate body (compressed message) - random dictionary
    let (body, algo, body_dict) = encode_compress(message)?;

    // Dejavu detection - did the universe give us the same dictionary twice?
    let dejavu = !title_dict.is_empty() && title_dict == body_dict;

    // Footer with compression algo, and decode hint only on dejavu
    let footer = if dejavu {
        format!("[{}]\nwhoa. base-d --detect --decompress {}", algo, algo)
    } else {
        format!("[{}]", algo)
    };

    println!("Title:  {}", title);
    println!("Body:   {}", body);
    if dejavu {
        println!("Dejavu: {} (both used {})", dejavu, title_dict);
    }
    println!("Footer: {}", footer);

    // Commit
    git_commit(&title, &body, &footer)?;
    println!("Committed.");

    // Push if requested
    if push {
        git_push()?;
        println!("Pushed.");
    }

    Ok(())
}
