use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const GEOFF_PREFIX: &str = "**Geoff:**";
const SOREN_PREFIX: &str = "**Soren:**";

/// Manifest metadata for an archived session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub session_id: String,
    pub archived_at: DateTime<Utc>,
    pub session_start: DateTime<Utc>,
    pub session_end: DateTime<Utc>,
    pub project_path: Option<String>,
    pub message_count: usize,
    pub agent_count: usize,
    pub agents: Vec<AgentInfo>,
    pub size_bytes: u64,
    pub checksum: String,
    // v2 fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageInfo>>,
    // v3 fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_clean_transcript: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub file: String,
    pub messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub hash: String,
    pub media_type: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_tool_use_id: Option<String>,
}

/// Archive the current session to the codex
pub fn save_session(session_path: Option<String>, all: bool, clean: bool) -> Result<()> {
    if all {
        save_all_sessions(clean)?;
    } else {
        let path = resolve_session_path(session_path)?;
        archive_session(&path, clean)?;
    }
    Ok(())
}

/// List archived sessions
pub fn list_sessions(all: bool, json: bool) -> Result<()> {
    let codex_dir = get_codex_dir()?;

    if !codex_dir.exists() {
        if json {
            println!("[]");
        } else {
            println!("No archives found (codex directory doesn't exist)");
        }
        return Ok(());
    }

    let mut archives = collect_archives(&codex_dir)?;

    if archives.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No archives found");
        }
        return Ok(());
    }

    // Sort by archived_at (most recent first)
    archives.sort_by(|a, b| b.manifest.archived_at.cmp(&a.manifest.archived_at));

    // Filter incremental archives unless --all
    if !all {
        // Group by base name (without .N suffix) and keep only latest
        let mut latest_map: std::collections::HashMap<String, ArchiveEntry> =
            std::collections::HashMap::new();

        for archive in archives {
            let base_name = get_base_archive_name(&archive.dir_name);
            latest_map
                .entry(base_name)
                .and_modify(|existing| {
                    // Keep the one with higher incremental number or most recent
                    if archive.incremental > existing.incremental {
                        *existing = archive.clone();
                    }
                })
                .or_insert(archive);
        }

        archives = latest_map.into_values().collect();
        archives.sort_by(|a, b| b.manifest.archived_at.cmp(&a.manifest.archived_at));
    }

    if json {
        let json_archives: Vec<serde_json::Value> = archives
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.short_id,
                    "dir_name": a.dir_name,
                    "incremental": a.incremental,
                    "archived_at": a.manifest.archived_at.to_rfc3339(),
                    "session_id": a.manifest.session_id,
                    "message_count": a.manifest.message_count,
                    "agent_count": a.manifest.agent_count,
                    "size_bytes": a.manifest.size_bytes,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_archives)?);
    } else {
        // Print table
        println!(
            "{:<25} {:<20} {:<8} {:<8} {:<10}",
            "ARCHIVE", "ARCHIVED", "MESSAGES", "AGENTS", "SIZE"
        );
        println!("{}", "-".repeat(80));

        for archive in archives {
            let size_kb = archive.manifest.size_bytes / 1024;
            let incremental_suffix = if archive.incremental > 0 {
                format!(".{}", archive.incremental)
            } else {
                String::new()
            };

            println!(
                "{:<25} {:<20} {:<8} {:<8} {:<10}",
                format!("{}{}", archive.short_id, incremental_suffix),
                archive.manifest.archived_at.format("%Y-%m-%d %H:%M:%S"),
                archive.manifest.message_count,
                archive.manifest.agent_count,
                format!("{}KB", size_kb)
            );
        }
    }

    Ok(())
}

/// Read and display an archived session
pub fn read_session(
    id: String,
    human: bool,
    grep_pattern: Option<String>,
    include_agents: bool,
    json: bool,
    clean: bool,
) -> Result<()> {
    let codex_dir = get_codex_dir()?;
    let archive_dir = find_archive_by_id(&codex_dir, &id)?;

    if clean {
        let transcript_file = archive_dir.join("conversation.md");
        if !transcript_file.exists() {
            anyhow::bail!(
                "No clean transcript for archive '{}'. Re-save with --clean or run 'codex migrate --clean'.",
                id
            );
        }
        let content = fs::read_to_string(&transcript_file)?;
        print!("{}", content);
        return Ok(());
    }

    if json {
        // Output manifest as JSON
        let manifest_path = archive_dir.join("manifest.json");
        if manifest_path.exists() {
            let manifest_content = fs::read_to_string(&manifest_path)?;
            let manifest: Manifest = serde_json::from_str(&manifest_content)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        } else {
            anyhow::bail!("Manifest not found in archive");
        }
        return Ok(());
    }

    let session_file = archive_dir.join("session.jsonl");
    if !session_file.exists() {
        anyhow::bail!("Session file not found in archive");
    }

    let content = fs::read_to_string(&session_file)?;

    if let Some(pattern) = grep_pattern {
        // Filter lines matching pattern
        for line in content.lines() {
            if line.contains(&pattern) {
                println!("{}", line);
            }
        }
    } else if human {
        // Pretty-print human-readable format
        print_human_readable(&content)?;
    } else {
        // Raw JSONL
        print!("{}", content);
    }

    // Include agent transcripts if requested
    if include_agents {
        let agents_dir = archive_dir.join("agents");
        if agents_dir.exists() {
            for entry in fs::read_dir(&agents_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    println!(
                        "\n--- Agent: {} ---\n",
                        path.file_stem().unwrap().to_string_lossy()
                    );
                    let agent_content = fs::read_to_string(&path)?;
                    if human {
                        print_human_readable(&agent_content)?;
                    } else {
                        print!("{}", agent_content);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Search all archives for a pattern
pub fn search_archives(pattern: String, json: bool) -> Result<()> {
    let codex_dir = get_codex_dir()?;

    if !codex_dir.exists() {
        if json {
            println!("[]");
        } else {
            println!("No archives found");
        }
        return Ok(());
    }

    let archives = collect_archives(&codex_dir)?;

    if json {
        let mut results = Vec::new();
        for archive in archives {
            let session_file = codex_dir.join(&archive.dir_name).join("session.jsonl");
            if let Ok(content) = fs::read_to_string(&session_file)
                && content.contains(&pattern)
            {
                let matching_lines: Vec<serde_json::Value> = content
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains(&pattern))
                    .map(|(i, line)| {
                        serde_json::json!({
                            "line": i + 1,
                            "content": line,
                        })
                    })
                    .collect();
                results.push(serde_json::json!({
                    "archive_id": archive.short_id,
                    "file": session_file.display().to_string(),
                    "matches": matching_lines,
                }));
            }
        }
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for archive in archives {
            let session_file = codex_dir.join(&archive.dir_name).join("session.jsonl");
            if let Ok(content) = fs::read_to_string(&session_file)
                && content.contains(&pattern)
            {
                println!("Match in {}: {}", archive.short_id, session_file.display());
                // Print matching lines
                for (i, line) in content.lines().enumerate() {
                    if line.contains(&pattern) {
                        println!("  Line {}: {}", i + 1, line);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Migrate all v1 archives to v2 (extract images to files)
pub fn migrate_archives(dry_run: bool, verbose: bool, clean: bool) -> Result<()> {
    let codex_dir = get_codex_dir()?;

    if !codex_dir.exists() {
        println!("No archives found (codex directory doesn't exist)");
        return Ok(());
    }

    let archives = collect_archives(&codex_dir)?;

    if archives.is_empty() {
        println!("No archives found");
        return Ok(());
    }

    // --clean mode: generate conversation.md for archives that have session.jsonl but no transcript
    if clean {
        return migrate_clean_transcripts(&codex_dir, archives, dry_run, verbose);
    }

    // Find archives that need migration (version < 2 or missing version)
    let mut to_migrate = Vec::new();
    for archive in archives {
        if archive.manifest.version < 2 {
            to_migrate.push(archive);
        }
    }

    if to_migrate.is_empty() {
        println!("All archives are already v2! Nothing to migrate.");
        return Ok(());
    }

    println!("Found {} archive(s) to migrate", to_migrate.len());

    if dry_run {
        println!("\n[DRY RUN MODE - No changes will be made]\n");
    }

    let mut total_migrated = 0;
    let mut total_images = 0;
    let mut total_bytes_saved = 0u64;

    for archive in to_migrate {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");

        if !session_file.exists() {
            eprintln!(
                "Warning: session.jsonl not found in {}, skipping",
                archive.dir_name
            );
            continue;
        }

        if verbose {
            println!("Migrating archive: {}", archive.short_id);
        }

        if !dry_run {
            // Create backup of original session.jsonl
            let backup_file = archive_dir.join("session.jsonl.bak");
            fs::copy(&session_file, &backup_file).context("Failed to create backup")?;

            // Create images directory
            let images_dir = archive_dir.join("images");
            fs::create_dir_all(&images_dir)?;

            // Extract images from session.jsonl
            let session_content = fs::read_to_string(&session_file)?;
            let (modified_session_content, mut all_images) =
                extract_images_from_jsonl(&session_content, &images_dir)?;

            // Write back modified session.jsonl
            fs::write(&session_file, modified_session_content)?;

            // Process agent files if they exist
            let agents_dir = archive_dir.join("agents");
            if agents_dir.exists() {
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();

                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        if verbose {
                            println!(
                                "  Processing agent file: {}",
                                path.file_name().unwrap().to_string_lossy()
                            );
                        }

                        // Backup agent file
                        let backup_path = path.with_extension("jsonl.bak");
                        fs::copy(&path, &backup_path)?;

                        // Extract images from agent file
                        let agent_content = fs::read_to_string(&path)?;
                        let (modified_agent_content, agent_images) =
                            extract_images_from_jsonl(&agent_content, &images_dir)?;

                        // Merge agent images (deduplicate)
                        for img in agent_images {
                            if !all_images.iter().any(|existing| existing.hash == img.hash) {
                                all_images.push(img);
                            }
                        }

                        // Write back modified agent file
                        fs::write(&path, modified_agent_content)?;
                    }
                }
            }

            // Calculate total bytes saved
            let bytes_saved: u64 = all_images.iter().map(|img| img.size_bytes).sum();
            total_bytes_saved += bytes_saved;

            // Update manifest to v2
            let mut manifest = archive.manifest.clone();
            manifest.version = 2;
            manifest.image_count = Some(all_images.len());
            manifest.images = Some(all_images.clone());

            let manifest_json = serde_json::to_string_pretty(&manifest)?;
            fs::write(archive_dir.join("manifest.json"), manifest_json)?;

            let image_count = all_images.len();
            total_images += image_count;

            if verbose || image_count > 0 {
                println!(
                    "  ✓ Migrated {}: {} images extracted, {} KB saved",
                    archive.short_id,
                    image_count,
                    bytes_saved / 1024
                );
            }
        } else {
            // Dry run - just count what would be migrated
            let session_content = fs::read_to_string(&session_file)?;
            let image_count = count_images_in_jsonl(&session_content)?;

            // Count images in agent files too
            let agents_dir = archive_dir.join("agents");
            let mut total_archive_images = image_count;

            if agents_dir.exists() {
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        let agent_content = fs::read_to_string(&path)?;
                        total_archive_images += count_images_in_jsonl(&agent_content)?;
                    }
                }
            }

            total_images += total_archive_images;

            if verbose || total_archive_images > 0 {
                println!(
                    "  Would migrate {}: {} images found",
                    archive.short_id, total_archive_images
                );
            }
        }

        total_migrated += 1;
    }

    println!("\n--- Migration Summary ---");
    println!("Archives migrated: {}", total_migrated);
    println!("Total images extracted: {}", total_images);

    if !dry_run {
        println!("Total space saved: {} KB", total_bytes_saved / 1024);
        println!("\n✓ Migration complete! Original files backed up as *.bak");
    } else {
        println!("\nRun without --dry-run to perform migration");
    }

    Ok(())
}

/// Count images in JSONL without extracting them (for dry-run)
fn count_images_in_jsonl(content: &str) -> Result<usize> {
    let mut count = 0;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = serde_json::from_str(line).context("Failed to parse JSONL line")?;

        count += count_images_in_value(&msg);
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

// --- Image extraction helpers ---

/// Extract and save images from a JSONL file, returning the modified content and image metadata
fn extract_images_from_jsonl(content: &str, images_dir: &Path) -> Result<(String, Vec<ImageInfo>)> {
    let mut images = Vec::new();
    let mut modified_lines = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            modified_lines.push(line.to_string());
            continue;
        }

        let mut msg: Value = serde_json::from_str(line).context("Failed to parse JSONL line")?;

        // Process the message content
        extract_images_from_value(&mut msg, images_dir, &mut images)?;

        modified_lines.push(serde_json::to_string(&msg)?);
    }

    Ok((modified_lines.join("\n") + "\n", images))
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
        _ => return Err(anyhow::anyhow!("Unsupported media type: {}", media_type)),
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

// --- Clean transcript helpers ---

/// Strip <system-reminder>...</system-reminder> blocks from a string
fn strip_system_reminders(content: &str) -> String {
    let re = Regex::new(r"(?s)<system-reminder>.*?</system-reminder>").unwrap();
    re.replace_all(content, "").to_string()
}

/// Generate a clean markdown transcript from JSONL session content
fn generate_clean_transcript(session_content: &str) -> Result<String> {
    let mut output = String::new();

    for line in session_content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines
        };

        let msg_type = match msg["type"].as_str() {
            Some(t) => t,
            None => continue,
        };

        match msg_type {
            "user" => {
                let content = &msg["message"]["content"];
                if let Some(text) = content.as_str() {
                    // String content: strip system reminders, skip if empty
                    let stripped = strip_system_reminders(text);
                    let trimmed = stripped.trim();
                    if !trimmed.is_empty() {
                        output.push_str(&format!("{} {}\n\n", GEOFF_PREFIX, trimmed));
                    }
                }
                // Array content (tool results): skip
            }
            "assistant" => {
                if let Some(blocks) = msg["message"]["content"].as_array() {
                    let mut text_parts = Vec::new();
                    for block in blocks {
                        if block["type"].as_str() == Some("text") {
                            if let Some(text) = block["text"].as_str() {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    text_parts.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                    let joined = text_parts.join("\n\n");
                    if !joined.is_empty() {
                        output.push_str(&format!("{} {}\n\n", SOREN_PREFIX, joined));
                    }
                }
            }
            _ => {} // skip summary, tool results, etc.
        }
    }

    Ok(output)
}

/// Generate clean transcripts for archives that have session.jsonl but no conversation.md
fn migrate_clean_transcripts(
    codex_dir: &Path,
    archives: Vec<ArchiveEntry>,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let mut needs_transcript = Vec::new();

    for archive in archives {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");
        let transcript_file = archive_dir.join("conversation.md");

        if transcript_file.exists() {
            // Already has a clean transcript — skip
            if verbose {
                println!("  Skipping {} (already has conversation.md)", archive.short_id);
            }
            continue;
        }

        if !session_file.exists() {
            // Clean-only archive or missing JSONL — can't generate
            if verbose {
                println!(
                    "  Skipping {} (no session.jsonl to generate from)",
                    archive.short_id
                );
            }
            continue;
        }

        needs_transcript.push(archive);
    }

    if needs_transcript.is_empty() {
        println!("All archives already have clean transcripts (or have no session.jsonl).");
        return Ok(());
    }

    println!(
        "Found {} archive(s) needing clean transcript",
        needs_transcript.len()
    );

    if dry_run {
        println!("\n[DRY RUN MODE - No changes will be made]\n");
        for archive in &needs_transcript {
            println!("  Would generate conversation.md for {}", archive.short_id);
        }
        return Ok(());
    }

    let mut generated = 0;

    for archive in &needs_transcript {
        let archive_dir = codex_dir.join(&archive.dir_name);
        let session_file = archive_dir.join("session.jsonl");
        let transcript_file = archive_dir.join("conversation.md");
        let manifest_path = archive_dir.join("manifest.json");

        let session_content = fs::read_to_string(&session_file)?;
        let transcript = generate_clean_transcript(&session_content)?;

        fs::write(&transcript_file, &transcript)?;

        // Update manifest to record has_clean_transcript
        if manifest_path.exists() {
            let manifest_content = fs::read_to_string(&manifest_path)?;
            if let Ok(mut manifest) = serde_json::from_str::<Manifest>(&manifest_content) {
                manifest.has_clean_transcript = Some(true);
                let updated = serde_json::to_string_pretty(&manifest)?;
                fs::write(&manifest_path, updated)?;
            }
        }

        if verbose {
            println!("  Generated conversation.md for {}", archive.short_id);
        }

        generated += 1;
    }

    println!("\n--- Migration Summary ---");
    println!("Clean transcripts generated: {}", generated);

    Ok(())
}

// --- Internal helpers ---

#[derive(Debug, Clone)]
struct ArchiveEntry {
    dir_name: String,
    short_id: String,
    incremental: u32,
    manifest: Manifest,
}

fn get_codex_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MX_CODEX_PATH") {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".crewu-private/logs/codex"))
}

fn resolve_session_path(path: Option<String>) -> Result<PathBuf> {
    if let Some(p) = path {
        Ok(PathBuf::from(p))
    } else {
        crate::session::find_most_recent_session()
    }
}

fn archive_session(session_path: &Path, clean: bool) -> Result<()> {
    if !session_path.exists() {
        anyhow::bail!("Session file not found: {:?}", session_path);
    }

    // Extract session metadata
    let session_id = session_path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid session filename")?
        .to_string();

    let metadata = fs::metadata(session_path)?;
    let modified = metadata.modified()?;
    let size_bytes = metadata.len();

    // Determine project path (parent directory name in .claude/projects/)
    let project_path = session_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    // Count messages
    let content = fs::read_to_string(session_path)?;
    let message_count = content.lines().filter(|l| !l.trim().is_empty()).count();

    // Calculate checksum
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let checksum = format!("sha256:{:x}", hasher.finalize());

    // Determine session start/end from file times
    let session_start: DateTime<Utc> = modified.into();
    let session_end: DateTime<Utc> = Utc::now();

    // Create archive directory
    let codex_dir = get_codex_dir()?;
    fs::create_dir_all(&codex_dir)?;

    // Generate archive directory name
    let short_uuid = &session_id[0..8.min(session_id.len())];
    let timestamp = session_start.format("%Y-%m-%d-%H%M%S");
    let base_name = format!("{}-{}", timestamp, short_uuid);

    // Check for existing archives and determine incremental suffix
    let archive_dir = determine_archive_dir(&codex_dir, &base_name)?;
    fs::create_dir_all(&archive_dir)?;

    if clean {
        // Clean mode: generate conversation.md + extract images — no JSONL, no agent file copies

        // Create images directory and extract images from session content
        let images_dir = archive_dir.join("images");
        fs::create_dir_all(&images_dir)?;

        let (_stripped_content, mut all_images) =
            extract_images_from_jsonl(&content, &images_dir)?;

        // Find associated agent sessions and extract images from them too (no file copy)
        let agents = find_agent_sessions(session_path, &modified)?;
        if !agents.is_empty() {
            for agent in &agents {
                let source_path = PathBuf::from(&agent.id);
                if let Ok(agent_content) = fs::read_to_string(&source_path) {
                    if let Ok((_modified_agent_content, agent_images)) =
                        extract_images_from_jsonl(&agent_content, &images_dir)
                    {
                        for img in agent_images {
                            if !all_images.iter().any(|existing| existing.hash == img.hash) {
                                all_images.push(img);
                            }
                        }
                    }
                }
            }
        }

        let image_count = all_images.len();

        // Generate clean transcript
        let transcript = generate_clean_transcript(&content)?;
        fs::write(archive_dir.join("conversation.md"), &transcript)?;

        let manifest = Manifest {
            version: 2,
            session_id: session_id.clone(),
            archived_at: Utc::now(),
            session_start,
            session_end,
            project_path,
            message_count,
            agent_count: 0,
            agents: Vec::new(),
            size_bytes,
            checksum,
            image_count: Some(image_count),
            images: Some(all_images),
            has_clean_transcript: Some(true),
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        fs::write(archive_dir.join("manifest.json"), manifest_json)?;

        println!("Archived session (clean) to: {}", archive_dir.display());
        println!("  Messages: {}", message_count);
        println!("  Images: {}", image_count);
        println!("  Size: {} KB", size_bytes / 1024);
        println!("  conversation.md written");

        return Ok(());
    }

    // Full mode (default): find agents, extract images, copy JSONL

    // Find associated agent sessions
    let agents = find_agent_sessions(session_path, &modified)?;

    // Create images directory for extracted images
    let images_dir = archive_dir.join("images");
    fs::create_dir_all(&images_dir)?;

    // Extract images from session file and save modified content
    let session_content = fs::read_to_string(session_path)?;
    let (modified_session_content, mut all_images) =
        extract_images_from_jsonl(&session_content, &images_dir)?;

    let dest_session = archive_dir.join("session.jsonl");
    fs::write(&dest_session, modified_session_content)?;

    // Copy agent files and extract images from them too
    if !agents.is_empty() {
        let agents_dir = archive_dir.join("agents");
        fs::create_dir_all(&agents_dir)?;

        for agent in &agents {
            let source_path = PathBuf::from(&agent.id);
            let agent_filename = source_path
                .file_name()
                .context("Agent path has no filename")?;
            let dest_agent = agents_dir.join(agent_filename);

            // Extract images from agent file
            let agent_content = fs::read_to_string(&source_path)?;
            let (modified_agent_content, agent_images) =
                extract_images_from_jsonl(&agent_content, &images_dir)?;

            // Merge agent images with all_images (deduplication handled by hash check)
            for img in agent_images {
                if !all_images.iter().any(|existing| existing.hash == img.hash) {
                    all_images.push(img);
                }
            }

            fs::write(&dest_agent, modified_agent_content)?;
        }
    }

    // Create manifest (v2 with image support)
    let image_count = all_images.len();
    let manifest = Manifest {
        version: 2,
        session_id: session_id.clone(),
        archived_at: Utc::now(),
        session_start,
        session_end,
        project_path,
        message_count,
        agent_count: agents.len(),
        agents: agents.clone(),
        size_bytes,
        checksum,
        image_count: Some(image_count),
        images: Some(all_images),
        has_clean_transcript: None,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(archive_dir.join("manifest.json"), manifest_json)?;

    println!("Archived session to: {}", archive_dir.display());
    println!("  Messages: {}", message_count);
    println!("  Agents: {}", agents.len());
    println!("  Images: {}", image_count);
    println!("  Size: {} KB", size_bytes / 1024);

    Ok(())
}

fn find_agent_sessions(
    session_path: &Path,
    _session_modified: &SystemTime,
) -> Result<Vec<AgentInfo>> {
    let parent_dir = session_path
        .parent()
        .context("Session file has no parent directory")?;

    let session_stem = session_path
        .file_stem()
        .context("Session file has no stem")?;

    // Construct path to subagents directory: {project}/<session_id>/subagents/
    let subagents_dir = parent_dir.join(session_stem).join("subagents");

    let mut agents = Vec::new();

    // Only search if subagents directory exists
    if subagents_dir.exists() {
        for entry in fs::read_dir(&subagents_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Check if it's an agent-*.jsonl file
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with("agent-")
                && path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            {
                // Check if modification time is within session window
                if let Ok(meta) = entry.metadata()
                    && let Ok(_modified) = meta.modified()
                {
                    // Simple heuristic: agent file modified around same time as session
                    // Could be improved with actual timestamp parsing from JSONL
                    let content = fs::read_to_string(&path)?;
                    let messages = content.lines().filter(|l| !l.trim().is_empty()).count();

                    agents.push(AgentInfo {
                        id: path.to_string_lossy().to_string(), // Store full path temporarily
                        file: format!("agents/{}", name),
                        messages,
                    });
                }
            }
        }
    }

    Ok(agents)
}

fn determine_archive_dir(codex_dir: &Path, base_name: &str) -> Result<PathBuf> {
    let base_dir = codex_dir.join(base_name);

    if !base_dir.exists() {
        return Ok(base_dir);
    }

    // Find highest incremental number
    let mut max_incremental = 0;
    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with(base_name)
            && let Some(suffix) = name_str.strip_prefix(base_name)
            && let Some(num_str) = suffix.strip_prefix('.')
            && let Ok(num) = num_str.parse::<u32>()
        {
            max_incremental = max_incremental.max(num);
        }
    }

    Ok(codex_dir.join(format!("{}.{}", base_name, max_incremental + 1)))
}

fn collect_archives(codex_dir: &Path) -> Result<Vec<ArchiveEntry>> {
    let mut archives = Vec::new();

    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }

        let manifest_content = fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = serde_json::from_str(&manifest_content)?;

        let dir_name = path.file_name().unwrap().to_string_lossy().to_string();
        let (short_id, incremental) = parse_archive_name(&dir_name);

        archives.push(ArchiveEntry {
            dir_name,
            short_id,
            incremental,
            manifest,
        });
    }

    Ok(archives)
}

fn parse_archive_name(name: &str) -> (String, u32) {
    // Extract short UUID from name like "2026-01-03-141500-abc12345" or "2026-01-03-141500-abc12345.1"
    if let Some(dot_pos) = name.rfind('.')
        && let Ok(num) = name[dot_pos + 1..].parse::<u32>()
    {
        let base = &name[..dot_pos];
        let short_id = extract_short_id(base);
        return (short_id, num);
    }

    (extract_short_id(name), 0)
}

fn extract_short_id(name: &str) -> String {
    // Extract last part after last hyphen (the short UUID)
    name.split('-').next_back().unwrap_or(name).to_string()
}

fn get_base_archive_name(name: &str) -> String {
    // Strip incremental suffix (.N) if present
    if let Some(dot_pos) = name.rfind('.')
        && name[dot_pos + 1..].parse::<u32>().is_ok()
    {
        return name[..dot_pos].to_string();
    }
    name.to_string()
}

fn find_archive_by_id(codex_dir: &Path, id: &str) -> Result<PathBuf> {
    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let name = path.file_name().unwrap().to_string_lossy();
        if name.contains(id) {
            return Ok(path);
        }
    }

    anyhow::bail!("Archive not found for id: {}", id)
}

fn print_human_readable(content: &str) -> Result<()> {
    use serde_json::Value;

    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse line {}", i + 1))?;

        let msg_type = msg["type"].as_str().unwrap_or("unknown");

        match msg_type {
            "user" => {
                if let Some(content) = msg["message"]["content"].as_str() {
                    println!("--- User ---");
                    println!("{}\n", content);
                }
            }
            "assistant" => {
                if let Some(blocks) = msg["message"]["content"].as_array() {
                    println!("--- Assistant ---");
                    for block in blocks {
                        if let Some(text) = block["text"].as_str() {
                            println!("{}", text);
                        } else if let Some(tool) = block["name"].as_str() {
                            println!("[Tool: {}]", tool);
                        }
                    }
                    println!();
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn save_all_sessions(clean: bool) -> Result<()> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let projects_dir = home.join(".claude").join("projects");

    if !projects_dir.exists() {
        anyhow::bail!("Claude projects directory not found");
    }

    let codex_dir = get_codex_dir()?;
    fs::create_dir_all(&codex_dir)?;

    // Collect all archived session IDs
    let mut archived_ids = std::collections::HashSet::new();
    if codex_dir.exists() {
        for entry in fs::read_dir(&codex_dir)? {
            let entry = entry?;
            let manifest_path = entry.path().join("manifest.json");
            if manifest_path.exists() {
                let content = fs::read_to_string(&manifest_path)?;
                let manifest: Manifest = serde_json::from_str(&content)?;
                archived_ids.insert(manifest.session_id);
            }
        }
    }

    // Scan for unarchived sessions
    let mut archived_count = 0;

    for entry in fs::read_dir(&projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        for file_entry in fs::read_dir(&path)? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();

            if file_path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }

            // Skip agent sessions
            if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("agent-") {
                    continue;
                }

                let session_id = name.trim_end_matches(".jsonl");
                if !archived_ids.contains(session_id) {
                    println!("Archiving: {}", session_id);
                    archive_session(&file_path, clean)?;
                    archived_count += 1;
                }
            }
        }
    }

    println!("Archived {} new session(s)", archived_count);

    Ok(())
}
