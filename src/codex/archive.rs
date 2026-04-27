use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::images::extract_images_from_jsonl;
use super::transcript::{
    build_agent_type_map, generate_clean_transcript, generate_clean_transcript_with_agents,
    resolve_agent_display_name, resolve_assistant_name, resolve_user_name,
};
use super::{AgentInfo, ArchiveEntry, Manifest};

/// Archive the current session to the codex
pub(crate) fn save_session(
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

pub(super) fn collect_archives(codex_dir: &Path) -> Result<Vec<ArchiveEntry>> {
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

pub(super) fn get_base_archive_name(name: &str) -> String {
    // Strip incremental suffix (.N) if present
    if let Some(dot_pos) = name.rfind('.')
        && name[dot_pos + 1..].parse::<u32>().is_ok()
    {
        return name[..dot_pos].to_string();
    }
    name.to_string()
}

pub(super) fn get_codex_dir() -> Result<PathBuf> {
    Ok(crate::paths::codex_dir())
}

fn save_all_sessions(clean: bool, include_agents: bool) -> Result<()> {
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
