use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::transcript::{
    build_agent_type_map, generate_clean_transcript, resolve_agent_display_name,
    resolve_assistant_name, resolve_user_name,
};
use super::{ArchiveEntry, Manifest};

use super::archive::{collect_archives, get_base_archive_name, get_codex_dir};

/// List archived sessions
pub(crate) fn list_sessions(all: bool, json: bool) -> Result<()> {
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
pub(crate) fn read_session(
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
pub(super) fn archive_source(archive_dir: &Path) -> Option<PathBuf> {
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
pub(crate) fn search_archives(pattern: String, json: bool) -> Result<()> {
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
