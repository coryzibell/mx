//! Encoded commit functionality - the upload pattern
//!
//! Commits are encoded for maximum entropy:
//! - Title: Hash of diff, encoded with random dictionary
//! - Body: Message compressed and encoded with random dictionary
//! - Footer: Compression algorithm hint
//!
//! Dejavu detection: When both title and body randomly get the same
//! dictionary, we add "whoa." to the footer.

use anyhow::{bail, Context, Result};
use base_d::prelude::*;
use std::process::Command;

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

/// Detect which dictionary was used to encode text
fn detect_dictionary(encoded: &str) -> Result<String> {
    let matches = base_d::detect_dictionary(encoded).map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(matches.first().map(|m| m.name.clone()).unwrap_or_default())
}

/// Encode text using base-d with hash and random dictionary
/// Returns (encoded_text, hash_algorithm)
pub fn encode_hash(text: &str) -> Result<(String, String)> {
    let registry = DictionaryRegistry::load_default()
        .map_err(|e| anyhow::anyhow!("Failed to load dictionaries: {}", e))?;

    let result = hash_encode(text.as_bytes(), &registry)
        .map_err(|e| anyhow::anyhow!("Hash encode failed: {}", e))?;

    Ok((result.encoded, result.hash_algo.as_str().to_string()))
}

/// Compress and encode text using base-d, returns (encoded, compress_algo)
pub fn encode_compress(text: &str) -> Result<(String, String)> {
    let registry = DictionaryRegistry::load_default()
        .map_err(|e| anyhow::anyhow!("Failed to load dictionaries: {}", e))?;

    let result = compress_encode(text.as_bytes(), &registry)
        .map_err(|e| anyhow::anyhow!("Compress encode failed: {}", e))?;

    Ok((result.encoded, result.compress_algo.as_str().to_string()))
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
            && !stderr.contains("no tracking information")
        {
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

/// Encoded commit parts
pub struct EncodedCommit {
    pub title: String,
    pub body: String,
    pub footer: String,
    pub dejavu: bool,
    pub title_dict: String,
}

impl EncodedCommit {
    /// Full commit message: title\n\nbody\n\nfooter
    pub fn message(&self) -> String {
        format!("{}\n\n{}\n\n{}", self.title, self.body, self.footer)
    }
}

/// Encode title and body into commit parts
/// Title is hashed, body is compressed, footer shows algos/dicts used
pub fn encode_commit(title_text: &str, body_text: &str) -> Result<EncodedCommit> {
    // Generate title (hash) - random dictionary
    let (title, hash_algo) = encode_hash(title_text)?;

    // Generate body (compressed) - random dictionary
    let (body, compress_algo) = encode_compress(body_text)?;

    // Detect dictionaries
    let title_dict = detect_dictionary(&title)?;
    let body_dict = detect_dictionary(&body)?;

    // Dejavu detection - same dictionary for both?
    let dejavu = !title_dict.is_empty() && !body_dict.is_empty() && title_dict == body_dict;

    // Footer: [hash_algo:title_dict|compress_algo:body_dict]
    let footer = format!(
        "[{}:{}|{}:{}]{}",
        hash_algo,
        title_dict,
        compress_algo,
        body_dict,
        if dejavu { "\nwhoa." } else { "" }
    );

    Ok(EncodedCommit {
        title,
        body,
        footer,
        dejavu,
        title_dict,
    })
}

/// Generate an encoded commit message from title and body
/// Returns the full message ready to use (title\n\nbody\n\nfooter)
pub fn encode_commit_message(title_text: &str, body_text: &str) -> Result<String> {
    Ok(encode_commit(title_text, body_text)?.message())
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

    // Get diff for hashing (title is hash of diff)
    let diff = get_staged_diff()?;

    // Encode: title from diff hash, body from compressed message
    let encoded = encode_commit(&diff, message)?;

    println!("Title:  {}", encoded.title);
    println!("Body:   {}", encoded.body);
    if encoded.dejavu {
        println!("Dejavu: true (both used {})", encoded.title_dict);
    }
    println!("Footer: {}", encoded.footer);

    // Commit
    git_commit(&encoded.title, &encoded.body, &encoded.footer)?;
    println!("Committed.");

    // Push if requested
    if push {
        git_push()?;
        println!("Pushed.");
    }

    Ok(())
}

/// Get PR diff via gh
fn get_pr_diff(number: u32) -> Result<String> {
    let output = Command::new("gh")
        .args(["pr", "diff", &number.to_string()])
        .output()
        .context("Failed to run gh pr diff")?;

    if !output.status.success() {
        bail!(
            "gh pr diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Merge a pull request with encoded commit message
pub fn pr_merge(number: u32, rebase: bool, merge_commit: bool) -> Result<()> {
    // Get PR diff for title hash
    let diff = get_pr_diff(number)?;

    // Get PR info from gh
    let pr_info = Command::new("gh")
        .args(["pr", "view", &number.to_string(), "--json", "title,body"])
        .output()
        .context("Failed to run gh pr view")?;

    if !pr_info.status.success() {
        bail!(
            "gh pr view failed: {}",
            String::from_utf8_lossy(&pr_info.stderr)
        );
    }

    // Parse JSON response
    let json: serde_json::Value =
        serde_json::from_slice(&pr_info.stdout).context("Failed to parse PR info")?;

    let pr_title = json["title"].as_str().unwrap_or("PR");
    let pr_body = json["body"].as_str().unwrap_or("");

    // Combine PR title and body into full message for body encoding
    let full_message = format!("{}\n\n{}", pr_title, pr_body);

    // Encode: title from diff hash, body from compressed full message
    let encoded = encode_commit(&diff, &full_message)?;

    // Determine merge method
    let method = if rebase {
        "rebase"
    } else if merge_commit {
        "merge"
    } else {
        "squash"
    };

    // Merge with gh - pass encoded title and body+footer separately
    let body_with_footer = format!("{}\n\n{}", encoded.body, encoded.footer);
    let output = Command::new("gh")
        .args([
            "pr",
            "merge",
            &number.to_string(),
            &format!("--{}", method),
            "--subject",
            &encoded.title,
            "--body",
            &body_with_footer,
        ])
        .output()
        .context("Failed to run gh pr merge")?;

    if !output.status.success() {
        bail!(
            "gh pr merge failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    println!("Merged PR #{} ({})", number, method);
    println!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}
