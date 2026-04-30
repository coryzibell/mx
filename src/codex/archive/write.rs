//! Per-session archive writer + the `--all` driver.
//!
//! This is the body that historically lived inline in `archive.rs`.
//! Logic is unchanged; only the file boundary moved.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::super::images::extract_images_from_jsonl;
use super::super::transcript::{
    build_agent_type_map, generate_clean_transcript, generate_clean_transcript_with_agents,
    resolve_agent_display_name, resolve_assistant_name, resolve_user_name,
};
use super::super::{MANIFEST_WRITE_VERSION, Manifest};
use super::get_codex_dir;
use super::paths::determine_archive_dir;
use super::sources::find_agent_sessions;

/// Archive a single session JSONL into a fresh codex directory.
pub(crate) fn archive_session(
    session_path: &Path,
    clean: bool,
    include_agents: bool,
) -> Result<()> {
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
            version: MANIFEST_WRITE_VERSION,
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
            // v5 sidecars not written yet (later commits in PR 2 wire these).
            tool_output_count: None,
            mcp_log_count: None,
            history_lines: None,
            source_breakdown: None,
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

    // Create manifest (schema v5; payload identical to v2 plus a higher
    // version number until later commits wire the new sidecars).
    let image_count = all_images.len();
    let manifest = Manifest {
        version: MANIFEST_WRITE_VERSION,
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
        // v5 sidecars not written yet (later commits in PR 2 wire these).
        tool_output_count: None,
        mcp_log_count: None,
        history_lines: None,
        source_breakdown: None,
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

/// Bulk-archive every unarchived session under `~/.claude/projects/`.
pub(crate) fn save_all_sessions(clean: bool, include_agents: bool) -> Result<()> {
    let projects_dir = crate::paths::claude_projects_dir();

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
