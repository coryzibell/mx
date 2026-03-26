//! Encoded commit functionality - the upload pattern
//!
//! Commits are encoded for maximum entropy:
//! - Title: Hash of diff, encoded with random dictionary
//! - Body: Message compressed and encoded with random dictionary
//! - Footer: Compression algorithm hint
//!
//! Dejavu detection: When both title and body randomly get the same
//! dictionary, we add "whoa." to the footer.

use anyhow::{Context, Result, bail};
use base_d::prelude::*;
use std::process::Command;

/// Maximum number of encoding attempts before giving up.
/// Each attempt re-rolls the random dictionary selection.
const MAX_ENCODE_ATTEMPTS: usize = 5;

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

/// Encode text using base-d with hash and random dictionary
/// Returns (encoded_text, hash_algorithm, dictionary_name)
pub fn encode_hash_with_registry(
    text: &str,
    registry: &DictionaryRegistry,
) -> Result<(String, String, String)> {
    let result = hash_encode(text.as_bytes(), registry)
        .map_err(|e| anyhow::anyhow!("Hash encode failed: {}", e))?;

    Ok((
        result.encoded,
        result.hash_algo.as_str().to_string(),
        result.dictionary_name,
    ))
}

/// Compress and encode text using base-d, returns (encoded, compress_algo, dictionary_name)
pub fn encode_compress_with_registry(
    text: &str,
    registry: &DictionaryRegistry,
) -> Result<(String, String, String)> {
    let result = compress_encode(text.as_bytes(), registry)
        .map_err(|e| anyhow::anyhow!("Compress encode failed: {}", e))?;

    Ok((
        result.encoded,
        result.compress_algo.as_str().to_string(),
        result.dictionary_name,
    ))
}

/// Decode and decompress text that was encoded with encode_compress
/// Footer format: [hash_algo:dict|compress_algo:dict]
pub fn decode_body(encoded: &str, footer: &str) -> Result<String> {
    use base_d::{CompressionAlgorithm, decode, decompress};

    let encoded = encoded.trim();

    // Parse footer to get compression algorithm
    let compress_algo = parse_compress_algo(footer);

    // Auto-detect dictionary and decode
    let matches = base_d::detect_dictionary(encoded).map_err(|e| anyhow::anyhow!("{}", e))?;

    if matches.is_empty() {
        bail!("Could not detect dictionary for encoded text");
    }

    // DictionaryMatch includes the dictionary itself
    let dict = &matches[0].dictionary;

    // Decode
    let decoded_bytes =
        decode(encoded, dict).map_err(|e| anyhow::anyhow!("Decode failed: {}", e))?;

    // Decompress if we have a compression algorithm
    let final_bytes = if let Some(algo) = compress_algo {
        let compression_algo = match algo.to_lowercase().as_str() {
            "lzma" => CompressionAlgorithm::Lzma,
            "zstd" => CompressionAlgorithm::Zstd,
            "brotli" => CompressionAlgorithm::Brotli,
            "gzip" | "gz" => CompressionAlgorithm::Gzip,
            "lz4" => CompressionAlgorithm::Lz4,
            "snappy" => CompressionAlgorithm::Snappy,
            _ => return String::from_utf8(decoded_bytes).context("Not valid UTF-8"),
        };
        decompress(&decoded_bytes, compression_algo)
            .map_err(|e| anyhow::anyhow!("Decompression failed: {}", e))?
    } else {
        decoded_bytes
    };

    String::from_utf8(final_bytes).context("Decoded content is not valid UTF-8")
}

/// Parse compression algorithm from footer
/// Footer format: [hash_algo:dict|compress_algo:dict]
fn parse_compress_algo(footer: &str) -> Option<String> {
    // Look for pattern like [sha384:base62|lzma:uuencode]
    let footer = footer.trim();
    if !footer.starts_with('[') || !footer.contains('|') {
        return None;
    }

    // Extract the part after |
    let pipe_pos = footer.find('|')?;
    let after_pipe = &footer[pipe_pos + 1..];

    // Get the compression algo (before the colon)
    let colon_pos = after_pipe.find(':')?;
    let algo = &after_pipe[..colon_pos];

    Some(algo.to_string())
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
    pub body_dict: String,
}

impl EncodedCommit {
    /// Full commit message: title\n\nbody\n\nfooter
    pub fn message(&self) -> String {
        format!("{}\n\n{}\n\n{}", self.title, self.body, self.footer)
    }
}

/// Validates that encoded output is safe for use as a command-line argument.
/// Returns Ok(()) if safe, or Err with a description of the problem (position and character).
/// The error message does NOT include dictionary info -- that is handled by the retry loop.
fn validate_encoded_output(encoded: &str, context: &str) -> Result<()> {
    if let Some(pos) = encoded.find('\0') {
        bail!("NUL byte at position {} in {}", pos, context,);
    }
    // Check for C0 controls (except newline, tab) and C1 controls
    for (i, c) in encoded.char_indices() {
        let cp = c as u32;
        if (cp < 0x20 && cp != 0x0A && cp != 0x09) || (0x80..=0x9F).contains(&cp) {
            bail!(
                "control character U+{:04X} at position {} in {}",
                cp,
                i,
                context,
            );
        }
    }
    Ok(())
}

/// Encode title and body into commit parts with automatic retry on unsafe output.
///
/// Loads the dictionary registry once and retries up to MAX_ENCODE_ATTEMPTS times
/// if the encoded output contains NUL bytes or control characters. Each retry
/// re-rolls the random dictionary selection. Failed attempts are logged to stderr
/// with the dictionary/codec combo that produced unsafe output.
pub fn encode_commit(title_text: &str, body_text: &str) -> Result<EncodedCommit> {
    // Load registry once for all attempts
    let registry = DictionaryRegistry::load_default()
        .map_err(|e| anyhow::anyhow!("Failed to load dictionaries: {}", e))?;

    let mut failed_footers: Vec<String> = Vec::new();

    for attempt in 1..=MAX_ENCODE_ATTEMPTS {
        // Generate title (hash) - random dictionary
        let (title, hash_algo, title_dict) = encode_hash_with_registry(title_text, &registry)?;

        // Generate body (compressed) - random dictionary
        let (body, compress_algo, body_dict) = encode_compress_with_registry(body_text, &registry)?;

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

        // Validate all parts for unsafe characters
        let title_check = validate_encoded_output(&title, "title");
        let body_check = validate_encoded_output(&body, "body");
        let footer_check = validate_encoded_output(&footer, "footer");

        if let Err(e) = title_check.and(body_check).and(footer_check) {
            let footer_tag = format!(
                "[{}:{}|{}:{}]",
                hash_algo, title_dict, compress_algo, body_dict
            );
            if attempt < MAX_ENCODE_ATTEMPTS {
                eprintln!("Tried {}: {}, retrying...", footer_tag, e);
            } else {
                eprintln!("Tried {}: {}", footer_tag, e);
            }
            failed_footers.push(footer_tag);
            continue;
        }

        // Success
        if attempt > 1 {
            let footer_tag = format!(
                "[{}:{}|{}:{}]",
                hash_algo, title_dict, compress_algo, body_dict
            );
            eprintln!("Tried {}: OK", footer_tag);
        }

        return Ok(EncodedCommit {
            title,
            body,
            footer,
            dejavu,
            title_dict,
            body_dict,
        });
    }

    // All attempts failed
    bail!(
        "All {} encoding attempts produced unsafe output. Failed dictionaries: {}",
        MAX_ENCODE_ATTEMPTS,
        failed_footers.join(", ")
    )
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

    // Encode with retry: title from diff hash, body from compressed message
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

    // Encode with retry: title from diff hash, body from compressed full message
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_encoded_clean_ascii() {
        assert!(validate_encoded_output("hello world", "test").is_ok());
    }

    #[test]
    fn test_validate_encoded_nul_byte() {
        assert!(validate_encoded_output("hello\0world", "test").is_err());
    }

    #[test]
    fn test_validate_encoded_c0_control() {
        assert!(validate_encoded_output("hello\x01world", "test").is_err());
    }

    #[test]
    fn test_validate_encoded_c1_control() {
        assert!(validate_encoded_output("hello\u{0085}world", "test").is_err());
    }

    #[test]
    fn test_validate_encoded_newline_allowed() {
        assert!(validate_encoded_output("hello\nworld", "test").is_ok());
    }

    #[test]
    fn test_validate_encoded_tab_allowed() {
        assert!(validate_encoded_output("hello\tworld", "test").is_ok());
    }

    #[test]
    fn test_validate_encoded_empty() {
        assert!(validate_encoded_output("", "test").is_ok());
    }

    #[test]
    fn test_validate_encoded_multibyte_unicode() {
        // Valid multi-byte chars should pass -- no false positives
        assert!(
            validate_encoded_output(
                "\u{1f711}\u{1f754}\u{1f72e}\u{1f716}\u{1f723}\u{1f75c}",
                "test"
            )
            .is_ok()
        );
    }
}
