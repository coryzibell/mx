use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use super::ImageInfo;

/// Parsing outcome for a JSONL stream. `skipped` counts non-empty lines
/// that failed JSON parsing. `parsed` counts non-empty lines that
/// succeeded.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct JsonlParseStats {
    pub parsed: usize,
    pub skipped: usize,
}

impl JsonlParseStats {
    pub(super) fn total(&self) -> usize {
        self.parsed + self.skipped
    }
}

fn preview_for_warning(line: &str) -> String {
    let prefix: String = line.chars().take(50).collect();
    prefix.escape_default().to_string()
}

fn format_skip_warning(line_num: usize, line: &str) -> String {
    format!(
        "warning: skipping invalid JSONL line {}: {}",
        line_num,
        preview_for_warning(line),
    )
}

/// Count images in JSONL without extracting them (for dry-run)
pub(super) fn count_images_in_jsonl(content: &str) -> Result<(usize, JsonlParseStats)> {
    let mut count = 0;
    let mut stats = JsonlParseStats::default();

    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(line) {
            Ok(msg) => {
                stats.parsed += 1;
                count += count_images_in_value(&msg);
            }
            Err(_) => {
                stats.skipped += 1;
                eprintln!("{}", format_skip_warning(idx + 1, line));
            }
        }
    }

    if stats.parsed == 0 && stats.skipped > 0 {
        anyhow::bail!(
            "All {} JSONL line(s) failed to parse — file appears corrupt",
            stats.skipped
        );
    }

    Ok((count, stats))
}

/// Recursively count images in JSON value
fn count_images_in_value(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            // Check if this is an image block
            if let Some(Value::String(type_val)) = map.get("type")
                && type_val == "image"
                && let Some(Value::Object(source)) = map.get("source")
                && let Some(Value::String(source_type)) = source.get("type")
                && source_type == "base64"
            {
                1
            } else {
                // Recursively count in all values
                map.values().map(count_images_in_value).sum()
            }
        }
        Value::Array(arr) => arr.iter().map(count_images_in_value).sum(),
        _ => 0,
    }
}

/// Extract and save images from a JSONL file, returning the modified content and image metadata
pub(super) fn extract_images_from_jsonl(
    content: &str,
    images_dir: &Path,
) -> Result<(String, Vec<ImageInfo>, JsonlParseStats)> {
    let mut images = Vec::new();
    let mut modified_lines = Vec::new();
    let mut stats = JsonlParseStats::default();

    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            modified_lines.push(line.to_string());
            continue;
        }

        let mut msg: Value = match serde_json::from_str(line) {
            Ok(v) => {
                stats.parsed += 1;
                v
            }
            Err(_) => {
                stats.skipped += 1;
                eprintln!("{}", format_skip_warning(idx + 1, line));
                // Drop the bad line from output — don't propagate corruption into the
                // archive. The eprintln warning above carries the forensics.
                continue;
            }
        };

        // Process the message content
        extract_images_from_value(&mut msg, images_dir, &mut images)?;

        modified_lines.push(serde_json::to_string(&msg)?);
    }

    if stats.parsed == 0 && stats.skipped > 0 {
        anyhow::bail!(
            "All {} JSONL line(s) failed to parse — file appears corrupt",
            stats.skipped
        );
    }

    Ok((modified_lines.join("\n") + "\n", images, stats))
}

/// Recursively walk JSON value and extract images
fn extract_images_from_value(
    value: &mut Value,
    images_dir: &Path,
    images: &mut Vec<ImageInfo>,
) -> Result<()> {
    match value {
        Value::Object(map) => {
            // Check if this is an image block
            if let Some(Value::String(type_val)) = map.get("type")
                && type_val == "image"
                && let Some(Value::Object(source)) = map.get("source")
                && let Some(Value::String(source_type)) = source.get("type")
                && source_type == "base64"
                && let Some(Value::String(media_type)) = source.get("media_type")
                && let Some(Value::String(data)) = source.get("data")
            {
                // Extract all needed data before we mutate
                let tool_use_id = map
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let media_type = media_type.clone();
                let data = data.clone();

                // Hash and save the image
                let (hash, size_bytes) = hash_image_data(&data)?;
                let file_ref = save_image(&data, &hash, &media_type, images_dir)?;

                // Add to images list if not already present
                if !images.iter().any(|img| img.hash == hash) {
                    images.push(ImageInfo {
                        hash: hash.clone(),
                        media_type: media_type.clone(),
                        size_bytes,
                        original_tool_use_id: tool_use_id,
                    });
                }

                // Now we can safely mutate the source
                if let Some(Value::Object(source)) = map.get_mut("source") {
                    source.clear();
                    source.insert("type".to_string(), Value::String("file".to_string()));
                    source.insert("file".to_string(), Value::String(file_ref));
                }
            } else {
                // Recursively process all values in the object
                for val in map.values_mut() {
                    extract_images_from_value(val, images_dir, images)?;
                }
            }
        }
        Value::Array(arr) => {
            // Recursively process all array elements
            for item in arr.iter_mut() {
                extract_images_from_value(item, images_dir, images)?;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Hash image data and return (hash, size_bytes)
fn hash_image_data(base64_data: &str) -> Result<(String, u64)> {
    let image_bytes = BASE64
        .decode(base64_data)
        .context("Failed to decode base64 image")?;

    let mut hasher = Sha256::new();
    hasher.update(&image_bytes);
    let hash = format!("{:x}", hasher.finalize());

    Ok((hash, image_bytes.len() as u64))
}

/// Save image to disk and return the file reference path
fn save_image(
    base64_data: &str,
    hash: &str,
    media_type: &str,
    images_dir: &Path,
) -> Result<String> {
    let image_bytes = BASE64
        .decode(base64_data)
        .context("Failed to decode base64 image")?;

    // Determine file extension from media type
    let ext = match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        unknown => {
            eprintln!(
                "Warning: unknown image media type '{}', saving as .bin",
                unknown
            );
            "bin"
        }
    };

    let filename = format!("{}.{}", hash, ext);
    let file_path = images_dir.join(&filename);

    // Only write if file doesn't exist (deduplication)
    if !file_path.exists() {
        fs::write(&file_path, image_bytes)
            .with_context(|| format!("Failed to write image file: {}", filename))?;
    }

    Ok(format!("images/{}", filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn valid_line(n: u32) -> String {
        format!(r#"{{"type":"user","seq":{}}}"#, n)
    }

    #[test]
    fn preview_truncates_to_50_chars() {
        let long = "x".repeat(200);
        let p = preview_for_warning(&long);
        // 50 chars of x escape to 50 chars (x is printable)
        assert_eq!(p.len(), 50);
    }

    #[test]
    fn preview_escapes_null_bytes() {
        let bad = "\u{0}\u{0}\u{0}".to_string();
        let p = preview_for_warning(&bad);
        assert!(p.contains("\\u{0}") || p.contains("\\0"));
    }

    #[test]
    fn preview_handles_multibyte_utf8() {
        // Make sure we never split a multi-byte char
        let s = "日".repeat(100);
        let p = preview_for_warning(&s);
        // Should not panic; should be 50 chars of 日 escaped
        assert!(!p.is_empty());
    }

    #[test]
    fn count_all_valid_lines_no_skip() {
        let content = format!("{}\n{}\n{}\n", valid_line(1), valid_line(2), valid_line(3));
        let (count, stats) = count_images_in_jsonl(&content).unwrap();
        assert_eq!(count, 0);
        assert_eq!(stats.parsed, 3);
        assert_eq!(stats.skipped, 0);
    }

    #[test]
    fn count_skips_invalid_line() {
        let content = format!("{}\nNOT JSON\n{}\n", valid_line(1), valid_line(2));
        let (_count, stats) = count_images_in_jsonl(&content).unwrap();
        assert_eq!(stats.parsed, 2);
        assert_eq!(stats.skipped, 1);
    }

    #[test]
    fn count_all_invalid_returns_err() {
        let content = "not json\nalso not json\n";
        let err = count_images_in_jsonl(content).unwrap_err();
        assert!(err.to_string().contains("All 2 JSONL"));
    }

    #[test]
    fn count_empty_content_is_ok() {
        let (count, stats) = count_images_in_jsonl("").unwrap();
        assert_eq!(count, 0);
        assert_eq!(stats.parsed, 0);
        assert_eq!(stats.skipped, 0);
    }

    #[test]
    fn count_all_blank_lines_is_ok() {
        let (count, stats) = count_images_in_jsonl("\n\n\n").unwrap();
        assert_eq!(count, 0);
        assert_eq!(stats.parsed, 0);
        assert_eq!(stats.skipped, 0);
    }

    #[test]
    fn extract_skips_invalid_line_and_drops_from_output() {
        let dir = TempDir::new().unwrap();
        let content = format!("{}\nNOT JSON\n{}\n", valid_line(1), valid_line(2));
        let (output, _images, stats) = extract_images_from_jsonl(&content, dir.path()).unwrap();
        assert_eq!(stats.parsed, 2);
        assert_eq!(stats.skipped, 1);
        assert!(!output.contains("NOT JSON"));
        assert!(output.contains(r#""seq":1"#));
        assert!(output.contains(r#""seq":2"#));
    }

    #[test]
    fn extract_null_byte_line_is_skipped() {
        let dir = TempDir::new().unwrap();
        // Exactly the bug from issue #178
        let null_line = "\u{0}".repeat(100);
        let content = format!("{}\n{}\n{}\n", valid_line(1), null_line, valid_line(2));
        let (output, _images, stats) = extract_images_from_jsonl(&content, dir.path()).unwrap();
        assert_eq!(stats.parsed, 2);
        assert_eq!(stats.skipped, 1);
        assert!(!output.contains('\u{0}'));
    }

    #[test]
    fn extract_all_invalid_returns_err() {
        let dir = TempDir::new().unwrap();
        let content = "garbage\nmore garbage\n";
        let err = extract_images_from_jsonl(content, dir.path()).unwrap_err();
        assert!(err.to_string().contains("All 2 JSONL"));
    }

    #[test]
    fn extract_empty_content_is_ok() {
        let dir = TempDir::new().unwrap();
        let (output, images, stats) = extract_images_from_jsonl("", dir.path()).unwrap();
        assert_eq!(stats.parsed, 0);
        assert_eq!(stats.skipped, 0);
        assert_eq!(output, "\n");
        assert!(images.is_empty());
    }

    #[test]
    fn warning_includes_one_indexed_line_number() {
        // Bad line at position 27 in a 50-line file
        let mut lines: Vec<String> = (0..50).map(|i| valid_line(i as u32)).collect();
        lines[26] = "NOT JSON".to_string(); // position 27 is index 26
        let content = lines.join("\n");

        // Verify the helper produces the expected text directly
        let warning = format_skip_warning(27, "NOT JSON");
        assert_eq!(warning, "warning: skipping invalid JSONL line 27: NOT JSON");

        // And verify the full path produces the right stats
        let (_count, stats) = count_images_in_jsonl(&content).unwrap();
        assert_eq!(stats.parsed, 49);
        assert_eq!(stats.skipped, 1);
    }
}
