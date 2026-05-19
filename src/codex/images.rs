use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use super::ImageInfo;

/// Return a sanitised preview of a line for terminal-safe warning output.
fn preview_line(line: &str) -> String {
    line.chars()
        .take(50)
        .collect::<String>()
        .escape_default()
        .to_string()
}

/// Count images in JSONL without extracting them (for dry-run)
pub(super) fn count_images_in_jsonl(content: &str) -> Result<usize> {
    let mut count = 0;
    let mut total_non_empty = 0usize;
    let mut skipped = 0usize;

    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        total_non_empty += 1;

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "warning: skipping invalid JSONL line {}: {}",
                    idx + 1,
                    preview_line(line)
                );
                skipped += 1;
                continue;
            }
        };

        count += count_images_in_value(&msg);
    }

    if total_non_empty > 0 && skipped == total_non_empty {
        anyhow::bail!(
            "All {} JSONL lines failed to parse — file is corrupt",
            skipped
        );
    }

    Ok(count)
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

/// Extract and save images from a JSONL file, returning the modified content,
/// image metadata, and the count of skipped (invalid) lines.
pub(super) fn extract_images_from_jsonl(
    content: &str,
    images_dir: &Path,
) -> Result<(String, Vec<ImageInfo>, usize)> {
    let mut images = Vec::new();
    let mut modified_lines = Vec::new();
    let mut total_non_empty = 0usize;
    let mut skipped = 0usize;

    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            modified_lines.push(line.to_string());
            continue;
        }

        total_non_empty += 1;

        let mut msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "warning: skipping invalid JSONL line {}: {}",
                    idx + 1,
                    preview_line(line)
                );
                skipped += 1;
                continue;
            }
        };

        // Process the message content
        extract_images_from_value(&mut msg, images_dir, &mut images)?;

        modified_lines.push(serde_json::to_string(&msg)?);
    }

    if total_non_empty > 0 && skipped == total_non_empty {
        anyhow::bail!(
            "All {} JSONL lines failed to parse — file is corrupt",
            skipped
        );
    }

    Ok((modified_lines.join("\n") + "\n", images, skipped))
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
    use tempfile::tempdir;

    #[test]
    fn extract_images_from_jsonl_skips_invalid_line() {
        let tmp = tempdir().unwrap();
        let content = "{\"type\":\"text\"}\n\0\0\0\n";
        let result = extract_images_from_jsonl(content, tmp.path());
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let (modified, _images, skipped) = result.unwrap();
        assert_eq!(skipped, 1, "expected 1 skipped line");
        assert!(
            modified.contains("{\"type\":\"text\"}"),
            "modified content should contain the valid line"
        );
        assert!(
            !modified.contains('\0'),
            "modified content must not contain null bytes"
        );
    }

    #[test]
    fn extract_images_from_jsonl_fails_when_all_invalid() {
        let tmp = tempdir().unwrap();
        let content = "\0\0\0\nnot json at all\n";
        let result = extract_images_from_jsonl(content, tmp.path());
        assert!(result.is_err(), "expected Err when all lines are invalid");
        let err = result.unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("failed to parse"),
            "error message should contain 'failed to parse', got: {}",
            msg
        );
    }

    #[test]
    fn extract_images_from_jsonl_empty_content() {
        let tmp = tempdir().unwrap();
        let result = extract_images_from_jsonl("", tmp.path());
        assert!(result.is_ok(), "expected Ok for empty content");
        let (_modified, _images, skipped) = result.unwrap();
        assert_eq!(skipped, 0, "expected 0 skipped for empty content");
    }

    #[test]
    fn extract_images_from_jsonl_only_empty_lines() {
        let tmp = tempdir().unwrap();
        let result = extract_images_from_jsonl("\n\n\n", tmp.path());
        assert!(result.is_ok(), "expected Ok for only-empty-lines content");
        let (_modified, _images, skipped) = result.unwrap();
        assert_eq!(skipped, 0, "empty lines must not be counted as skipped");
    }

    #[test]
    fn count_images_in_jsonl_skips_invalid_line() {
        let content = "{\"type\":\"text\"}\n\0\0\0\n";
        let result = count_images_in_jsonl(content);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        assert_eq!(result.unwrap(), 0, "expected 0 images in valid text line");
    }

    #[test]
    fn count_images_in_jsonl_fails_when_all_invalid() {
        let content = "\0\0\0\nnot json at all\n";
        let result = count_images_in_jsonl(content);
        assert!(result.is_err(), "expected Err when all lines are invalid");
    }
}
