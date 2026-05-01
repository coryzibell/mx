//! Encoded commit functionality - the upload pattern
//!
//! Commits are encoded for maximum entropy:
//! - Title: Hash of diff, encoded with random dictionary
//! - Body: Message compressed and encoded with random dictionary
//! - Footer: Compression algorithm hint
//!
//! Dejavu detection: When both title and body randomly get the same
//! dictionary, we add the `DEJAVU_MARKER` to the footer.

use anyhow::{Context, Result, bail};
use base_d::prelude::*;
use std::process::Command;

/// Maximum number of encoding attempts before giving up.
/// Each attempt re-rolls the random dictionary selection.
const MAX_ENCODE_ATTEMPTS: usize = 5;

/// Marker line appended to the footer when title and body randomly land
/// on the same encoding dictionary (the dejavu easter egg).
///
/// Single source of truth: the encoder writes this exact line, and the
/// `mx log` rendering filter in `handlers::try_decode_commit_body`
/// strips this exact line from displayed bodies. Both sides import
/// this constant -- if the spelling ever changes, both update together.
pub(crate) const DEJAVU_MARKER: &str = "whoa.";

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
fn encode_hash_with_registry(
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
fn encode_compress_with_registry(
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

/// Map a compression-algorithm name (as it appears in the footer) to the
/// `base_d::CompressionAlgorithm` enum. Returns `None` if the name is not
/// in our known vocabulary.
///
/// This is the single source of truth for which compression names are
/// considered "real" footer compression algorithms. Both `decode_body`
/// (for actual decompression) and `is_known_compress_algo` (for footer
/// validation in `is_footer_line`) consult this set; lifting it here
/// keeps the vocabulary from drifting between the two call sites.
///
/// Note: the names tracked here mirror what `base_d::CompressionAlgorithm::as_str()`
/// emits, NOT the broader set that `base_d::CompressionAlgorithm::from_str` accepts
/// as aliases (e.g. `zst`, `br`, `snap`, `xz`). The encoder always serializes
/// canonical names, so the validator only needs to recognize those. If `base_d`
/// ever changes which name it emits, update this map; the walking test
/// `is_footer_line_accepts_each_known_algo` will catch a mismatch loudly.
pub(crate) fn compression_algo_from_str(s: &str) -> Option<base_d::CompressionAlgorithm> {
    use base_d::CompressionAlgorithm;
    match s.to_lowercase().as_str() {
        "lzma" => Some(CompressionAlgorithm::Lzma),
        "zstd" => Some(CompressionAlgorithm::Zstd),
        "brotli" => Some(CompressionAlgorithm::Brotli),
        "gzip" | "gz" => Some(CompressionAlgorithm::Gzip),
        "lz4" => Some(CompressionAlgorithm::Lz4),
        "snappy" => Some(CompressionAlgorithm::Snappy),
        _ => None,
    }
}

/// Returns true if `s` names a compression algorithm we recognize as
/// belonging to a real footer. Used by `is_footer_line` (in handlers) to
/// distinguish a real footer from any user-authored bracket-pipe text
/// that happens to satisfy the structural shape.
pub(crate) fn is_known_compress_algo(s: &str) -> bool {
    compression_algo_from_str(s).is_some()
}

/// Decode and decompress text that was encoded with encode_compress
/// Footer format: [hash_algo:dict|compress_algo:dict]
pub fn decode_body(encoded: &str, footer: &str) -> Result<String> {
    use base_d::{DictionaryRegistry, decode, decompress};

    let encoded = encoded.trim();

    // Parse footer to get compression algorithm and body dictionary name
    let compress_algo = parse_compress_algo(footer);
    let body_dict_name = parse_body_dict(footer);

    // Look up dictionary by name from footer, fall back to auto-detection
    // for old commits that may lack a proper footer
    let dict = if let Some(ref dict_name) = body_dict_name {
        let registry = DictionaryRegistry::load_default()
            .map_err(|e| anyhow::anyhow!("Failed to load dictionary registry: {}", e))?;
        registry
            .dictionary(dict_name)
            .map_err(|e| anyhow::anyhow!("Dictionary '{}' not found: {}", dict_name, e))?
    } else {
        // Backward compat: no dict in footer, fall back to auto-detection
        let matches = base_d::detect_dictionary(encoded).map_err(|e| anyhow::anyhow!("{}", e))?;
        if matches.is_empty() {
            bail!("Could not detect dictionary for encoded text");
        }
        matches[0].dictionary.clone()
    };

    // Decode
    let decoded_bytes =
        decode(encoded, &dict).map_err(|e| anyhow::anyhow!("Decode failed: {}", e))?;

    // Decompress if we have a compression algorithm
    let final_bytes = if let Some(algo) = compress_algo {
        let compression_algo = match compression_algo_from_str(&algo) {
            Some(a) => a,
            None => return String::from_utf8(decoded_bytes).context("Not valid UTF-8"),
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
pub(crate) fn parse_compress_algo(footer: &str) -> Option<String> {
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

/// Parse body dictionary name from footer
/// Footer format: [hash_algo:title_dict|compress_algo:body_dict]
pub(crate) fn parse_body_dict(footer: &str) -> Option<String> {
    let footer = footer.trim();
    if !footer.starts_with('[') || !footer.contains('|') {
        return None;
    }

    // Extract the part after |
    let pipe_pos = footer.find('|')?;
    let after_pipe = &footer[pipe_pos + 1..];

    // Get the body dict name (after the colon, before the closing bracket)
    let colon_pos = after_pipe.find(':')?;
    let after_colon = &after_pipe[colon_pos + 1..];

    // Strip trailing ']' and anything after (e.g., newline + `DEJAVU_MARKER`)
    let dict_name = after_colon.split(']').next()?;

    if dict_name.is_empty() {
        return None;
    }

    Some(dict_name.to_string())
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

/// Format the footer tag: `[hash_algo:title_dict|compress_algo:body_dict]`
fn format_footer_tag(
    hash_algo: &str,
    title_dict: &str,
    compress_algo: &str,
    body_dict: &str,
) -> String {
    format!(
        "[{}:{}|{}:{}]",
        hash_algo, title_dict, compress_algo, body_dict
    )
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
        let footer_tag = format_footer_tag(&hash_algo, &title_dict, &compress_algo, &body_dict);
        let footer = format!(
            "{}{}",
            footer_tag,
            if dejavu {
                format!("\n{}", DEJAVU_MARKER)
            } else {
                String::new()
            }
        );

        // Validate all parts for unsafe characters
        let title_check = validate_encoded_output(&title, "title");
        let body_check = validate_encoded_output(&body, "body");
        let footer_check = validate_encoded_output(&footer, "footer");

        if let Err(e) = title_check.and(body_check).and(footer_check) {
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

/// Format an `EncodedCommit` for human-facing stdout display.
///
/// - `show_encoded == false` (default): returns only the `Footer:` line.
///   The title and body are random-glyph noise with a freshly-rolled
///   dictionary per commit, so they are useless to a human at stdout.
///   The footer identifies the hash/compression/dictionary combo, which
///   IS meaningful confirmation that encoding succeeded.
///
/// - `show_encoded == true`: returns the full dump (`Title:`, `Body:`,
///   `Dejavu:` when applicable, `Footer:`), matching the historical
///   behavior of `upload_commit` verbatim.
///
/// The returned string does NOT include a trailing newline or the
/// `Committed.` / `Pushed.` status lines — those are the caller's
/// responsibility. Kept as a pure function so tests can assert on the
/// exact output without spawning a subprocess.
pub fn format_encoded_commit(encoded: &EncodedCommit, show_encoded: bool) -> String {
    let mut out = String::new();
    if show_encoded {
        out.push_str(&format!("Title:  {}\n", encoded.title));
        out.push_str(&format!("Body:   {}\n", encoded.body));
        if encoded.dejavu {
            out.push_str(&format!(
                "Dejavu: true (both used {})\n",
                encoded.title_dict
            ));
        }
    }
    out.push_str(&format!("Footer: {}", encoded.footer));
    out
}

/// Prefix every line of `output` with `[dry-run] `.
///
/// Useful for marking preview output so it is never mistaken for a real
/// commit log. Extracted as a standalone helper so it can be unit-tested
/// without git state.
pub fn prefix_dry_run(output: &str) -> String {
    output
        .lines()
        .map(|line| format!("[dry-run] {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Perform the full upload commit.
///
/// `show_encoded` controls stdout verbosity:
/// - `false` (default): prints only the footer line and `Committed.`
///   (plus `Pushed.` if `push` is set).
/// - `true`: prints the full `Title:` / `Body:` / `Dejavu:` / `Footer:`
///   block — historical behavior, opt-in via `mx commit --show-encoded`.
///
/// `dry_run` runs all encoding/validation logic but skips the actual git
/// operations (commit, push). Output is prefixed with `[dry-run]` so it
/// is never mistaken for a real commit log. Exits 0 on success, nonzero
/// if the real commit would have failed (no staged changes, encoding
/// error, etc.).
pub fn upload_commit(
    message: &str,
    stage_all_flag: bool,
    push: bool,
    show_encoded: bool,
    dry_run: bool,
) -> Result<()> {
    // Stage if requested — but never under dry-run: mutating the index
    // violates the dry-run contract.
    if stage_all_flag && !dry_run {
        stage_all()?;
    }

    // Check for staged changes
    if !has_staged_changes()? {
        if dry_run {
            bail!("[dry-run] No staged changes to commit");
        }
        bail!("No staged changes to commit");
    }

    // Get diff for hashing (title is hash of diff)
    let diff = get_staged_diff()?;

    // Encode with retry: title from diff hash, body from compressed message
    let encoded = encode_commit(&diff, message)?;

    if dry_run {
        let formatted = format_encoded_commit(&encoded, show_encoded);
        let mut preview = formatted;
        preview.push_str("\nWould commit.");
        if push {
            preview.push_str("\nWould push.");
        }
        println!("{}", prefix_dry_run(&preview));
        return Ok(());
    }

    println!("{}", format_encoded_commit(&encoded, show_encoded));

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

    // --- format_encoded_commit ---

    fn sample_encoded_no_dejavu() -> EncodedCommit {
        EncodedCommit {
            title: "TTTT-title-glyphs".to_string(),
            body: "BBBB-body-glyphs".to_string(),
            footer: "[sha384:base62|lzma:uuencode]".to_string(),
            dejavu: false,
            title_dict: "base62".to_string(),
            body_dict: "uuencode".to_string(),
        }
    }

    fn sample_encoded_with_dejavu() -> EncodedCommit {
        EncodedCommit {
            title: "TTTT-title-glyphs".to_string(),
            body: "BBBB-body-glyphs".to_string(),
            footer: format!("[sha384:base62|lzma:base62]\n{}", DEJAVU_MARKER),
            dejavu: true,
            title_dict: "base62".to_string(),
            body_dict: "base62".to_string(),
        }
    }

    #[test]
    fn test_format_default_omits_title_and_body() {
        let encoded = sample_encoded_no_dejavu();
        let out = format_encoded_commit(&encoded, false);
        assert!(
            !out.contains("Title:"),
            "default output must not contain Title: -- got {:?}",
            out
        );
        assert!(
            !out.contains("Body:"),
            "default output must not contain Body: -- got {:?}",
            out
        );
        assert!(
            !out.contains("Dejavu:"),
            "default output must not contain Dejavu: -- got {:?}",
            out
        );
    }

    #[test]
    fn test_format_default_contains_footer() {
        let encoded = sample_encoded_no_dejavu();
        let out = format_encoded_commit(&encoded, false);
        assert!(
            out.contains("Footer: [sha384:base62|lzma:uuencode]"),
            "default output must contain the footer line -- got {:?}",
            out
        );
    }

    #[test]
    fn test_format_default_dejavu_still_hidden() {
        // Even when dejavu is true, default mode hides everything but footer.
        let encoded = sample_encoded_with_dejavu();
        let out = format_encoded_commit(&encoded, false);
        assert!(!out.contains("Dejavu:"));
        assert!(!out.contains("Title:"));
        assert!(!out.contains("Body:"));
        assert!(out.contains("Footer:"));
    }

    #[test]
    fn test_format_verbose_contains_all_fields() {
        let encoded = sample_encoded_no_dejavu();
        let out = format_encoded_commit(&encoded, true);
        assert!(out.contains("Title:  TTTT-title-glyphs"));
        assert!(out.contains("Body:   BBBB-body-glyphs"));
        assert!(out.contains("Footer: [sha384:base62|lzma:uuencode]"));
        // No dejavu on this sample, so the line should NOT appear.
        assert!(!out.contains("Dejavu:"));
    }

    #[test]
    fn test_format_verbose_shows_dejavu_when_true() {
        let encoded = sample_encoded_with_dejavu();
        let out = format_encoded_commit(&encoded, true);
        assert!(out.contains("Title:  TTTT-title-glyphs"));
        assert!(out.contains("Body:   BBBB-body-glyphs"));
        assert!(out.contains("Dejavu: true (both used base62)"));
        assert!(out.contains("Footer: [sha384:base62|lzma:base62]"));
    }

    #[test]
    fn test_format_verbose_exact_bytes_match_historical_output() {
        // Historical order (before this change) was Title, Body,
        // optional Dejavu, Footer. Keep that order exact so `--show-encoded`
        // is a byte-for-byte match of pre-refactor stdout. Asserting on the
        // full string (not just substring order) catches any drift in
        // spacing, field labels, or separators -- two formatters that
        // happened to interleave the fields in the right order but with
        // different whitespace would have passed the old substring check.
        let encoded = sample_encoded_with_dejavu();
        let out = format_encoded_commit(&encoded, true);
        let expected = format!(
            "Title:  TTTT-title-glyphs\n\
             Body:   BBBB-body-glyphs\n\
             Dejavu: true (both used base62)\n\
             Footer: [sha384:base62|lzma:base62]\n{}",
            DEJAVU_MARKER
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn test_format_no_trailing_newline() {
        // Caller adds its own newline via println!; the formatter must not
        // double-space the output.
        let encoded = sample_encoded_no_dejavu();
        let out = format_encoded_commit(&encoded, false);
        assert!(!out.ends_with('\n'));
        let out_v = format_encoded_commit(&encoded, true);
        assert!(!out_v.ends_with('\n'));
    }

    // --- parse_body_dict ---

    #[test]
    fn test_parse_body_dict_standard_footer() {
        assert_eq!(
            parse_body_dict("[sha384:base62|lzma:uuencode]"),
            Some("uuencode".to_string())
        );
    }

    #[test]
    fn test_parse_body_dict_base58_variant() {
        assert_eq!(
            parse_body_dict("[sha256:base64|gzip:base58ripple]"),
            Some("base58ripple".to_string())
        );
    }

    #[test]
    fn test_parse_body_dict_dejavu_footer() {
        // Footer may have trailing content after ']' on next lines
        let footer = format!("[sha384:base62|lzma:base62]\n{}", DEJAVU_MARKER);
        assert_eq!(parse_body_dict(&footer), Some("base62".to_string()));
    }

    #[test]
    fn test_parse_body_dict_no_footer() {
        assert_eq!(parse_body_dict("not a footer"), None);
    }

    #[test]
    fn test_parse_body_dict_empty() {
        assert_eq!(parse_body_dict(""), None);
    }

    #[test]
    fn test_parse_body_dict_no_pipe() {
        assert_eq!(parse_body_dict("[sha384:base62]"), None);
    }

    #[test]
    fn test_parse_body_dict_no_colon_after_pipe() {
        assert_eq!(parse_body_dict("[sha384:base62|lzma]"), None);
    }

    // --- parse_compress_algo ---

    #[test]
    fn test_parse_compress_algo_standard() {
        assert_eq!(
            parse_compress_algo("[sha384:base62|lzma:uuencode]"),
            Some("lzma".to_string())
        );
    }

    #[test]
    fn test_parse_compress_algo_none() {
        assert_eq!(parse_compress_algo("not a footer"), None);
    }

    // --- is_known_compress_algo / compression_algo_from_str ---

    #[test]
    fn test_is_known_compress_algo_accepts_canonical() {
        // The canonical vocabulary the encoder actually emits.
        for name in ["lzma", "zstd", "brotli", "gzip", "lz4", "snappy"] {
            assert!(
                is_known_compress_algo(name),
                "expected {} to be a known algo",
                name
            );
        }
    }

    #[test]
    fn test_is_known_compress_algo_accepts_gz_alias() {
        assert!(is_known_compress_algo("gz"));
    }

    #[test]
    fn test_is_known_compress_algo_is_case_insensitive() {
        assert!(is_known_compress_algo("LZMA"));
        assert!(is_known_compress_algo("Zstd"));
    }

    #[test]
    fn test_is_known_compress_algo_rejects_unknown() {
        assert!(!is_known_compress_algo("notreal"));
        assert!(!is_known_compress_algo(""));
        assert!(!is_known_compress_algo("anything"));
    }

    // --- prefix_dry_run ---

    #[test]
    fn test_prefix_dry_run_single_line() {
        let out = prefix_dry_run("Footer: [sha384:base62|lzma:uuencode]");
        assert_eq!(out, "[dry-run] Footer: [sha384:base62|lzma:uuencode]");
    }

    #[test]
    fn test_prefix_dry_run_multi_line() {
        let input = "Title:  TTTT\nBody:   BBBB\nFooter: [x:y|z:w]";
        let out = prefix_dry_run(input);
        for line in out.lines() {
            assert!(
                line.starts_with("[dry-run] "),
                "every line must start with [dry-run] prefix -- got {:?}",
                line,
            );
        }
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn test_prefix_dry_run_empty_input() {
        // An empty string has zero lines, so the prefixed result is also empty.
        let out = prefix_dry_run("");
        assert_eq!(out, "");
    }

    // --- dry-run formatted output ---

    #[test]
    fn test_dry_run_output_format_default() {
        // Simulate what upload_commit builds for dry-run (default mode):
        // format_encoded_commit + status lines, then prefix_dry_run.
        let encoded = sample_encoded_no_dejavu();
        let formatted = format_encoded_commit(&encoded, false);
        let mut preview = formatted;
        preview.push_str("\nWould commit.");
        let prefixed = prefix_dry_run(&preview);

        let lines: Vec<&str> = prefixed.lines().collect();
        // Should have: footer line + "Would commit."
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("[dry-run] Footer:"));
        assert_eq!(lines[1], "[dry-run] Would commit.");
    }

    #[test]
    fn test_dry_run_output_format_with_push() {
        let encoded = sample_encoded_no_dejavu();
        let formatted = format_encoded_commit(&encoded, false);
        let mut preview = formatted;
        preview.push_str("\nWould commit.");
        preview.push_str("\nWould push.");
        let prefixed = prefix_dry_run(&preview);

        let lines: Vec<&str> = prefixed.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("[dry-run] Footer:"));
        assert_eq!(lines[1], "[dry-run] Would commit.");
        assert_eq!(lines[2], "[dry-run] Would push.");
    }

    #[test]
    fn test_dry_run_output_format_verbose() {
        // With show_encoded == true, more lines appear before the status.
        let encoded = sample_encoded_with_dejavu();
        let formatted = format_encoded_commit(&encoded, true);
        let mut preview = formatted;
        preview.push_str("\nWould commit.");
        let prefixed = prefix_dry_run(&preview);

        for line in prefixed.lines() {
            assert!(
                line.starts_with("[dry-run] "),
                "every line must have [dry-run] prefix -- got {:?}",
                line,
            );
        }
        // Title, Body, Dejavu, Footer (with dejavu marker on same footer value),
        // Would commit.
        assert!(prefixed.contains("[dry-run] Title:  TTTT-title-glyphs"));
        assert!(prefixed.contains("[dry-run] Body:   BBBB-body-glyphs"));
        assert!(prefixed.contains("[dry-run] Dejavu: true (both used base62)"));
        assert!(prefixed.contains("[dry-run] Would commit."));
    }
}
