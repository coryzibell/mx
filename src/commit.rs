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
use rand::prelude::IndexedRandom;
use std::io::Write;
use std::process::{Command, Stdio};

/// Get available compression algorithms from base-d config
fn get_compress_algos(base_d: &str) -> Result<Vec<String>> {
    let output = Command::new(base_d)
        .args(["config", "--compression"])
        .output()
        .context("Failed to run base-d config")?;

    if !output.status.success() {
        bail!(
            "base-d config failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let algos: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .trim()
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    if algos.is_empty() {
        bail!("No compression algorithms returned by base-d config");
    }

    Ok(algos)
}

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

/// Find base-d binary (cached)
fn find_base_d() -> Result<&'static str> {
    use std::sync::OnceLock;
    static BASE_D: OnceLock<Option<String>> = OnceLock::new();

    let cached = BASE_D.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            format!("{}/.cargo/bin/base-d", home),
            "base-d".to_string(),
            format!("{}/.local/bin/base-d", home),
        ];

        for candidate in &candidates {
            if Command::new("which")
                .arg(candidate)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(candidate.clone());
            }

            if std::path::Path::new(candidate).exists() {
                return Some(candidate.clone());
            }
        }

        // Try mise
        let mise_path = format!("{}/.local/share/mise/installs/cargo-base-d", home);
        if let Ok(entries) = std::fs::read_dir(&mise_path) {
            for entry in entries.flatten() {
                let bin_path = entry.path().join("bin/base-d");
                if bin_path.exists() {
                    return Some(bin_path.to_string_lossy().to_string());
                }
            }
        }

        None
    });

    cached
        .as_ref()
        .map(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("base-d not found. Install with: cargo install base-d"))
}

/// Run base-d with args and stdin input, return output
fn run_base_d(args: &[&str], input: &str) -> Result<std::process::Output> {
    let base_d = find_base_d()?;

    let mut child = Command::new(base_d)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn base-d")?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .context("Failed to write to base-d stdin")?;

    child
        .wait_with_output()
        .context("Failed to wait for base-d")
}

/// Detect which dictionary was used to encode text
fn detect_dictionary(encoded: &str) -> Result<String> {
    let output = run_base_d(&["--detect"], encoded)?;

    // --detect prints "Detected: <name> (confidence: XX%)" to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    let dict = stderr
        .lines()
        .find(|l| l.starts_with("Detected:"))
        .and_then(|l| l.strip_prefix("Detected:"))
        .and_then(|s| s.split('(').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    Ok(dict)
}

/// Encode text using base-d with hash and random dictionary
/// Returns (encoded_text, hash_algorithm)
pub fn encode_hash(text: &str) -> Result<(String, String)> {
    let output = run_base_d(&["--hash", "--dejavu"], text)?;

    if !output.status.success() {
        bail!(
            "base-d hash failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Parse hash algorithm from stderr: "Note: Using randomly selected hash 'md5'"
    let stderr = String::from_utf8_lossy(&output.stderr);
    let hash_algo = stderr
        .lines()
        .find(|l| l.contains("Using randomly selected hash"))
        .and_then(|l| l.split('\'').nth(1))
        .unwrap_or("unknown")
        .to_string();

    let encoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((encoded, hash_algo))
}

/// Compress and encode text using base-d, returns (encoded, compress_algo)
pub fn encode_compress(text: &str) -> Result<(String, String)> {
    let algos = get_compress_algos(find_base_d()?)?;
    let algo = algos
        .choose(&mut rand::rng())
        .context("No compression algorithms available")?;

    let output = run_base_d(&["--compress", algo, "--dejavu"], text)?;

    if !output.status.success() {
        bail!(
            "base-d compress failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let encoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((encoded, algo.to_string()))
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

/// Merge a pull request with encoded commit message
pub fn pr_merge(number: u32, rebase: bool, merge_commit: bool) -> Result<()> {
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

    // Generate encoded commit message
    let encoded = encode_commit_message(pr_title, pr_body)?;

    // Determine merge method
    let method = if rebase {
        "rebase"
    } else if merge_commit {
        "merge"
    } else {
        "squash"
    };

    // Merge with gh
    let output = Command::new("gh")
        .args([
            "pr",
            "merge",
            &number.to_string(),
            &format!("--{}", method),
            "--body",
            &encoded,
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
