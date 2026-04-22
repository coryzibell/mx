mod images;
mod transcript;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use images::{count_images_in_jsonl, extract_images_from_jsonl};
use transcript::{
    build_agent_type_map, generate_clean_transcript, generate_clean_transcript_with_agents,
    migrate_clean_transcripts, resolve_agent_display_name, resolve_assistant_name,
    resolve_user_name,
};

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
    // v4 fields - configurable speaker names
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
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
pub fn save_session(
    session_path: Option<String>,
    all: bool,
    clean: bool,
    include_agents: bool,
) -> Result<()> {
    if all {
        save_all_sessions(clean, include_agents)?;
    } else {
        let path = resolve_session_path(session_path)?;
        archive_session(&path, clean, include_agents)?;
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
    archives.sort_by_key(|a| std::cmp::Reverse(a.manifest.archived_at));

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
        archives.sort_by_key(|a| std::cmp::Reverse(a.manifest.archived_at));
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
    clean_agents: bool,
) -> Result<()> {
    let codex_dir = get_codex_dir()?;
    let archive_dir = find_archive_by_id(&codex_dir, &id)?;

    // Load manifest once for use across all code paths that need it
    let manifest_path = archive_dir.join("manifest.json");
    let manifest: Option<Manifest> = if manifest_path.exists() {
        let mc = fs::read_to_string(&manifest_path)?;
        serde_json::from_str(&mc).ok()
    } else {
        None
    };

    if clean && !json {
        let transcript_file = archive_dir.join("conversation.md");
        if !transcript_file.exists() {
            anyhow::bail!(
                "No clean transcript for archive '{}'. Re-save with --clean or run 'codex migrate --clean'.",
                id
            );
        }
        let mut content = fs::read_to_string(&transcript_file)?;

        // If --agents requested but transcript doesn't contain agent sections,
        // attempt to generate them from agent JSONL files in the archive
        if clean_agents && !content.contains("\n## Agent: ") {
            let agents_dir = archive_dir.join("agents");
            if agents_dir.exists() {
                // Try to build agent type map from session.jsonl if available
                let session_file = archive_dir.join("session.jsonl");
                let agent_type_map = if session_file.exists() {
                    let sc = fs::read_to_string(&session_file).unwrap_or_default();
                    build_agent_type_map(&sc)
                } else {
                    HashMap::new()
                };
                let mut agent_sessions = Vec::new();
                for entry in fs::read_dir(&agents_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        let agent_name = resolve_agent_display_name(&path, &agent_type_map);
                        let agent_content = fs::read_to_string(&path)?;
                        agent_sessions.push((agent_name, agent_content));
                    }
                }
                agent_sessions.sort_by(|a, b| a.0.cmp(&b.0));
                // Resolve speaker names from manifest loaded at function entry
                let (r_user, r_asst) = (
                    manifest
                        .as_ref()
                        .and_then(|m| m.user_name.clone())
                        .unwrap_or_else(resolve_user_name),
                    manifest
                        .as_ref()
                        .and_then(|m| m.assistant_name.clone())
                        .unwrap_or_else(resolve_assistant_name),
                );
                for (agent_name, agent_content) in &agent_sessions {
                    let agent_transcript =
                        generate_clean_transcript(agent_content, &r_user, &r_asst)?;
                    if !agent_transcript.is_empty() {
                        content.push_str(&format!(
                            "\n---\n\n## Agent: {}\n\n{}",
                            agent_name, agent_transcript
                        ));
                    }
                }
            }
        }

        if let Some(pattern) = grep_pattern {
            for line in content.lines() {
                if line.contains(&pattern) {
                    println!("{}", line);
                }
            }
        } else {
            print!("{}", content);
        }
        return Ok(());
    }

    if json {
        // Output manifest as JSON (using pre-loaded manifest)
        if let Some(ref m) = manifest {
            println!("{}", serde_json::to_string_pretty(m)?);
        } else {
            anyhow::bail!("Manifest not found in archive");
        }
        return Ok(());
    }

    let source_file = archive_source(&archive_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "No transcript found in archive (expected conversation.md or session.jsonl)"
        )
    })?;
    let is_clean_md = source_file.extension().and_then(|e| e.to_str()) == Some("md");
    let content = fs::read_to_string(&source_file)?;

    if let Some(pattern) = grep_pattern {
        // Filter lines matching pattern — works identically on both formats
        for line in content.lines() {
            if line.contains(&pattern) {
                println!("{}", line);
            }
        }
    } else if !is_clean_md && human {
        // Pretty-print human-readable format (JSONL only)
        print_human_readable(&content)?;
    } else {
        // Raw output (JSONL) or clean markdown pass-through
        print!("{}", content);
    }

    // Include agent transcripts if requested.
    // Agent transcripts are always stored as JSONL (even in clean-mode archives
    // that use conversation.md for the main session), so we only look for .jsonl
    // files in the agents/ directory.
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

/// Pick the canonical transcript source for an archive directory.
/// Prefers the clean markdown transcript (post-March-31 default), falls back
/// to the raw JSONL for older archives.  Returns None when neither exists.
fn archive_source(archive_dir: &Path) -> Option<PathBuf> {
    let md = archive_dir.join("conversation.md");
    if md.exists() {
        return Some(md);
    }
    let jsonl = archive_dir.join("session.jsonl");
    if jsonl.exists() {
        return Some(jsonl);
    }
    None
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

    let mut skipped: usize = 0;

    if json {
        let mut results = Vec::new();
        for archive in archives {
            let archive_dir = codex_dir.join(&archive.dir_name);
            let source_file = match archive_source(&archive_dir) {
                Some(p) => p,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            match fs::read_to_string(&source_file) {
                Ok(content) => {
                    if content.contains(&pattern) {
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
                            "file": source_file.display().to_string(),
                            "matches": matching_lines,
                        }));
                    }
                }
                Err(e) => {
                    eprintln!("Warning: could not read {}: {}", source_file.display(), e);
                }
            }
        }
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for archive in archives {
            let archive_dir = codex_dir.join(&archive.dir_name);
            let source_file = match archive_source(&archive_dir) {
                Some(p) => p,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            match fs::read_to_string(&source_file) {
                Ok(content) => {
                    if content.contains(&pattern) {
                        println!("Match in {}: {}", archive.short_id, source_file.display());
                        for (i, line) in content.lines().enumerate() {
                            if line.contains(&pattern) {
                                println!("  Line {}: {}", i + 1, line);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: could not read {}: {}", source_file.display(), e);
                }
            }
        }
    }

    if skipped > 0 {
        eprintln!(
            "searched archives, skipped {} (no transcript found)",
            skipped
        );
    }

    Ok(())
}

/// Migrate all v1 archives to v2 (extract images to files)
pub fn migrate_archives(
    dry_run: bool,
    verbose: bool,
    clean: bool,
    include_agents: bool,
) -> Result<()> {
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
        return migrate_clean_transcripts(&codex_dir, archives, dry_run, verbose, include_agents);
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

// --- Internal helpers ---

#[derive(Debug, Clone)]
struct ArchiveEntry {
    dir_name: String,
    short_id: String,
    incremental: u32,
    manifest: Manifest,
}

fn get_codex_dir() -> Result<PathBuf> {
    Ok(crate::paths::codex_dir())
}

fn resolve_session_path(path: Option<String>) -> Result<PathBuf> {
    if let Some(p) = path {
        Ok(PathBuf::from(p))
    } else {
        crate::session::find_most_recent_session()
    }
}

fn archive_session(session_path: &Path, clean: bool, include_agents: bool) -> Result<()> {
    if !session_path.exists() {
        anyhow::bail!("Session file not found: {:?}", session_path);
    }

    // Resolve speaker names once for the entire function
    let user_name = resolve_user_name();
    let assistant_name = resolve_assistant_name();

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

        let (_stripped_content, mut all_images) = extract_images_from_jsonl(&content, &images_dir)?;

        // Find associated agent sessions and extract images from them too (no file copy)
        let agents = find_agent_sessions(session_path, &modified)?;
        if !agents.is_empty() {
            for agent in &agents {
                let source_path = PathBuf::from(&agent.id);
                if let Ok(agent_content) = fs::read_to_string(&source_path)
                    && let Ok((_modified_agent_content, agent_images)) =
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

        let image_count = all_images.len();

        // Generate clean transcript (optionally with agent conversations)
        let agent_type_map = build_agent_type_map(&content);
        let transcript = if include_agents && !agents.is_empty() {
            let mut agent_sessions = Vec::new();
            for agent in &agents {
                let source_path = PathBuf::from(&agent.id);
                if let Ok(agent_content) = fs::read_to_string(&source_path) {
                    let agent_name = resolve_agent_display_name(&source_path, &agent_type_map);
                    agent_sessions.push((agent_name, agent_content));
                }
            }
            agent_sessions.sort_by(|a, b| a.0.cmp(&b.0));
            generate_clean_transcript_with_agents(
                &content,
                &agent_sessions,
                &user_name,
                &assistant_name,
            )?
        } else {
            generate_clean_transcript(&content, &user_name, &assistant_name)?
        };
        let conversation_md_path = archive_dir.join("conversation.md");
        fs::write(&conversation_md_path, &transcript)?;

        // Compute actual archive size: conversation.md + all image files
        let md_size = fs::metadata(&conversation_md_path)
            .map(|m| m.len())
            .unwrap_or(transcript.len() as u64);
        let images_size: u64 = all_images.iter().map(|img| img.size_bytes).sum();
        let archive_size_bytes = md_size + images_size;

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
            size_bytes: archive_size_bytes,
            checksum,
            image_count: Some(image_count),
            images: Some(all_images),
            has_clean_transcript: Some(true),
            user_name: Some(user_name.clone()),
            assistant_name: Some(assistant_name.clone()),
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        fs::write(archive_dir.join("manifest.json"), manifest_json)?;

        println!("Archived session (clean) to: {}", archive_dir.display());
        println!("  Messages: {}", message_count);
        println!("  Images: {}", image_count);
        println!("  Size: {} KB", archive_size_bytes / 1024);
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
        user_name: Some(user_name),
        assistant_name: Some(assistant_name),
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

fn save_all_sessions(clean: bool, include_agents: bool) -> Result<()> {
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
                    archive_session(&file_path, clean, include_agents)?;
                    archived_count += 1;
                }
            }
        }
    }

    println!("Archived {} new session(s)", archived_count);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // W4: backwards-compat & edge-case tests
    // ---------------------------------------------------------------------------

    #[test]
    fn manifest_deserialize_without_speaker_names() {
        // Old archives will not have user_name or assistant_name fields.
        // Verify they deserialize as None for backwards compatibility.
        let json = r#"{
            "version": 2,
            "session_id": "abc123",
            "archived_at": "2026-01-01T00:00:00Z",
            "session_start": "2026-01-01T00:00:00Z",
            "session_end": "2026-01-01T00:00:00Z",
            "project_path": null,
            "message_count": 10,
            "agent_count": 0,
            "agents": [],
            "size_bytes": 1024,
            "checksum": "sha256:aaa"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.user_name, None);
        assert_eq!(manifest.assistant_name, None);
        assert_eq!(manifest.has_clean_transcript, None);
        assert_eq!(manifest.image_count, None);
    }

    // ---------------------------------------------------------------------------
    // archive_source: format-detection regression test
    // ---------------------------------------------------------------------------

    #[test]
    fn archive_source_prefers_clean_md_over_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("conversation.md");
        let jsonl_path = dir.path().join("session.jsonl");

        // Only conversation.md present (clean-mode archive)
        std::fs::write(&md_path, "# Conversation\n").unwrap();
        let result = archive_source(dir.path()).unwrap();
        assert_eq!(result, md_path, "should prefer conversation.md");

        // Both present — still prefers conversation.md
        std::fs::write(&jsonl_path, "{}\n").unwrap();
        let result = archive_source(dir.path()).unwrap();
        assert_eq!(
            result, md_path,
            "should prefer conversation.md when both exist"
        );

        // Only session.jsonl present (legacy archive)
        std::fs::remove_file(&md_path).unwrap();
        let result = archive_source(dir.path()).unwrap();
        assert_eq!(result, jsonl_path, "should fall back to session.jsonl");

        // Neither present — returns None
        std::fs::remove_file(&jsonl_path).unwrap();
        assert!(
            archive_source(dir.path()).is_none(),
            "should return None when no transcript exists"
        );
    }

    #[test]
    fn archive_source_real_archives_are_searchable() {
        // Cross-reference rail: at least one real archive must be in the canonical
        // format that archive_source can find.  This is the test that would have
        // caught the asymmetric migration at CI time — the check that clean-mode
        // archives are actually visible to the search path.
        let codex_dir = match get_codex_dir() {
            Ok(d) => d,
            Err(_) => return, // codex dir not configured in this environment — skip
        };
        if !codex_dir.exists() {
            return; // no archives yet — skip rather than fail
        }
        let archives = match collect_archives(&codex_dir) {
            Ok(a) => a,
            Err(_) => return,
        };
        if archives.is_empty() {
            return; // nothing to check
        }
        let searchable = archives.iter().filter(|a| {
            let archive_dir = codex_dir.join(&a.dir_name);
            archive_source(&archive_dir).is_some()
        });
        assert!(
            searchable.count() > 0,
            "no archives are searchable — archive_source found neither \
             conversation.md nor session.jsonl in any of the {} archive(s) \
             under {:?}. search_archives would return zero results.",
            archives.len(),
            codex_dir
        );
    }
}
