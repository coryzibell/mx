//! Per-session archive writer + the `--all` driver.
//!
//! This is the body that historically lived inline in `archive.rs`.
//! Logic is preserved; the only behavioral wiring change is that
//! subagent capture is now gated on `IncludeSet::subagents`. Status-quo
//! callers (`IncludeSet::status_quo()`) leave `subagents = true`, so
//! the default output is byte-identical to the pre-PR-2 implementation.

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
use super::super::{AgentInfo, MANIFEST_WRITE_VERSION, Manifest, SourceBreakdown};
use super::ArchiveResult;
use super::get_codex_dir;
use super::include::IncludeSet;
use super::paths::determine_archive_dir;
use super::sources::{
    TimestampWindow, derive_session_window, find_agent_sessions, find_history_slice, find_mcp_logs,
    find_tool_outputs,
};

/// Best-effort uid lookup via the `getuid(2)` syscall.
///
/// The previous heuristic stat-ed `$HOME` and read its uid, which is
/// fragile: `$HOME` can be set to a directory owned by a different user
/// (containers, sudo with HOME-preserved, dropped privileges in CI),
/// and on shared mounts the uid of the home directory may not match
/// the running process. A direct `getuid(2)` syscall is the only
/// authoritative answer.
///
/// We bind libc's `getuid` directly via `extern "C"` to avoid pulling
/// in `libc` or `nix` for a single integer. `getuid(2)` cannot fail per
/// POSIX, so the call is infallible — we still return `Option<u32>` to
/// keep the non-Unix shape and the caller's existing graceful-fallback
/// path intact.
fn current_uid() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: getuid(2) is documented to always succeed and have no
        // side effects. The raw FFI binding matches the POSIX prototype
        // (`uid_t getuid(void)`); on every libc Rust supports, `uid_t`
        // is a 32-bit unsigned integer.
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        Some(unsafe { getuid() })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Encode the home directory as Claude does: leading `-`, separator
/// `/` → `-`. E.g. `/home/charlie` → `-home-charlie`.
fn home_user_slug() -> Option<String> {
    let home = dirs::home_dir()?;
    let s = home.to_string_lossy();
    Some(s.replace('/', "-"))
}

/// Sum the byte sizes of every file under `archive_dir/agents/`.
/// Returns 0 if the directory does not exist (no agents captured).
fn total_agents_bytes(archive_dir: &Path) -> u64 {
    let agents_dir = archive_dir.join("agents");
    if !agents_dir.exists() {
        return 0;
    }
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(&agents_dir) {
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Build the optional `source_breakdown` field. Returns `None` when no
/// new (PR 2) sidecars were captured AND the legacy byte counts are
/// also zero, so status-quo archives stay byte-identical to v2/v3/v4
/// manifests (which never carried a breakdown).
fn build_source_breakdown(
    session_jsonl_bytes: u64,
    agents_bytes: u64,
    images_bytes: u64,
    sidecars: &SidecarCounts,
) -> Option<SourceBreakdown> {
    let any_new_sidecar = sidecars.mcp_log_count.is_some()
        || sidecars.tool_output_count.is_some()
        || sidecars.history_lines.is_some();
    if !any_new_sidecar {
        // Status-quo: don't emit the field at all. Keeps manifest output
        // byte-identical to the pre-PR-2 implementation.
        return None;
    }
    Some(SourceBreakdown {
        session_jsonl_bytes,
        agents_bytes,
        images_bytes,
        mcp_bytes: sidecars.mcp_bytes,
        tool_output_bytes: sidecars.tool_output_bytes,
        history_bytes: sidecars.history_bytes,
    })
}

/// Counts produced by the optional sidecar capture step.
#[derive(Debug, Default)]
struct SidecarCounts {
    tool_output_count: Option<usize>,
    mcp_log_count: Option<usize>,
    history_lines: Option<usize>,
    mcp_bytes: u64,
    tool_output_bytes: u64,
    history_bytes: u64,
}

/// Capture the optional new sidecars (MCP / tool-output / history)
/// according to `include`, writing each into a subdirectory of
/// `archive_dir`. Returns the per-source counts so the caller can
/// populate the manifest's v5 fields.
///
/// Each sub-walker is best-effort: failures inside one source are
/// logged to stderr and that source is left out of the archive,
/// rather than aborting the whole archive run. Archive success is the
/// load-bearing operation; the new sidecars are auxiliary.
///
/// `cwd_encoded` is the same encoding Claude uses (the `project_path`
/// stored on the manifest); `session_uuid` is the session_id.
fn capture_optional_sidecars(
    archive_dir: &Path,
    cwd_encoded: Option<&str>,
    session_uuid: &str,
    window: TimestampWindow,
    include: &IncludeSet,
) -> SidecarCounts {
    let mut counts = SidecarCounts::default();

    // ---- MCP logs ----
    if include.mcp
        && let Some(cwd) = cwd_encoded
    {
        match find_mcp_logs(cwd, window) {
            Ok(paths) => {
                let mcp_dir = archive_dir.join("mcp");
                if let Err(e) = fs::create_dir_all(&mcp_dir) {
                    eprintln!("warning: failed to create mcp/ sidecar dir: {e}");
                } else {
                    let mut n = 0usize;
                    let mut bytes = 0u64;
                    for src in paths {
                        let name = match src.file_name() {
                            Some(n) => n.to_owned(),
                            None => continue,
                        };
                        let dest = mcp_dir.join(&name);
                        if let Err(e) = fs::copy(&src, &dest) {
                            eprintln!("warning: failed to copy mcp log {src:?}: {e}");
                            continue;
                        }
                        if let Ok(meta) = fs::metadata(&dest) {
                            bytes += meta.len();
                        }
                        n += 1;
                    }
                    counts.mcp_log_count = Some(n);
                    counts.mcp_bytes = bytes;
                }
            }
            Err(e) => eprintln!("warning: mcp log walk failed: {e}"),
        }
    }

    // ---- Tool outputs ----
    if include.tool_output
        && let (Some(uid), Some(user_slug)) = (current_uid(), home_user_slug())
    {
        match find_tool_outputs(uid, &user_slug, session_uuid) {
            Ok(paths) => {
                let to_dir = archive_dir.join("tool-output");
                if let Err(e) = fs::create_dir_all(&to_dir) {
                    eprintln!("warning: failed to create tool-output/ sidecar dir: {e}");
                } else {
                    let mut n = 0usize;
                    let mut bytes = 0u64;
                    for src in paths {
                        let name = match src.file_name() {
                            Some(n) => n.to_owned(),
                            None => continue,
                        };
                        let dest = to_dir.join(&name);
                        if let Err(e) = fs::copy(&src, &dest) {
                            eprintln!("warning: failed to copy tool output {src:?}: {e}");
                            continue;
                        }
                        if let Ok(meta) = fs::metadata(&dest) {
                            bytes += meta.len();
                        }
                        n += 1;
                    }
                    counts.tool_output_count = Some(n);
                    counts.tool_output_bytes = bytes;
                }
            }
            Err(e) => eprintln!("warning: tool-output walk failed: {e}"),
        }
    }

    // ---- History slice ----
    if include.history {
        match find_history_slice(window) {
            Ok(lines) => {
                let history_dir = archive_dir.join("history");
                if let Err(e) = fs::create_dir_all(&history_dir) {
                    eprintln!("warning: failed to create history/ sidecar dir: {e}");
                } else {
                    let payload = if lines.is_empty() {
                        String::new()
                    } else {
                        let mut s = lines.join("\n");
                        s.push('\n');
                        s
                    };
                    let dest = history_dir.join("history.jsonl");
                    let bytes = payload.len() as u64;
                    if let Err(e) = fs::write(&dest, &payload) {
                        eprintln!("warning: failed to write history slice: {e}");
                    } else {
                        counts.history_lines = Some(lines.len());
                        counts.history_bytes = bytes;
                    }
                }
            }
            Err(e) => eprintln!("warning: history walk failed: {e}"),
        }
    }

    counts
}

/// Archive a single session JSONL into a fresh codex directory.
/// Returns the chosen archive directory.
///
/// N3: previously returned `Result<Option<PathBuf>>` "to leave room for
/// future no-op short-circuits." That short-circuit never materialized
/// (every code path either errors or returns `Some(dir)`), so the
/// `Option` was load-bearing only as a noise generator at every call
/// site. Tightened to `Result<PathBuf>`.
pub(crate) fn archive_session(
    session_path: &Path,
    clean: bool,
    include_agents_in_clean_md: bool,
    include: &IncludeSet,
) -> Result<PathBuf> {
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
    let size_bytes = metadata.len();

    // Determine the cwd-encoded project slug (the parent directory name
    // under .claude/projects/, e.g. `-home-charlie-recipes-coryzibell-mx`).
    //
    // NOTE: today this string is stored verbatim in `manifest.project_path`
    // because the encoding is identity at the manifest layer: the slug
    // IS the project_path (we don't decode it). A future decoder must
    // re-apply Claude's `/` -> `-` encoding before passing through to
    // MCP-log walkers, which require the cwd_encoded form.
    let cwd_encoded = session_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    let project_path = cwd_encoded.clone();

    // Count messages
    let content = fs::read_to_string(session_path)?;
    let message_count = content.lines().filter(|l| !l.trim().is_empty()).count();

    // Calculate checksum
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let checksum = format!("sha256:{:x}", hasher.finalize());

    // Derive the session window from the JSONL's first/last event
    // timestamps. This is the load-bearing input to MCP / history
    // attribution — using mtime alone (the pre-fix heuristic) misses
    // the actual session boundary by however long the user took to run
    // their last tool. `derive_session_window` falls back to file
    // metadata for empty/garbage JSONLs and warns on stderr.
    let window = derive_session_window(session_path)?;
    let session_start: DateTime<Utc> = window.start;
    let session_end: DateTime<Utc> = window.end;

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

    // Subagent capture is gated on the include set. Status-quo defaults
    // (`IncludeSet::status_quo`) leave this on, preserving the pre-PR-2
    // behavior; an explicit `--include none` (or any set without
    // `subagents`) suppresses it.
    let agents: Vec<AgentInfo> = if include.subagents {
        find_agent_sessions(session_path)?
    } else {
        Vec::new()
    };

    if clean {
        // Clean mode: generate conversation.md + extract images — no JSONL, no agent file copies
        let images_dir = archive_dir.join("images");
        fs::create_dir_all(&images_dir)?;

        let (_stripped_content, mut all_images) = extract_images_from_jsonl(&content, &images_dir)?;

        // Extract images from agent files too (no file copy in clean mode)
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
        let transcript = if include_agents_in_clean_md && !agents.is_empty() {
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

        // Capture optional new sidecars (gated on the include set).
        // Status-quo callers never trigger this — all three flags are
        // off by default, leaving the manifest byte-identical.
        let sidecars = capture_optional_sidecars(
            &archive_dir,
            cwd_encoded.as_deref(),
            &session_id,
            window,
            include,
        );
        let breakdown = build_source_breakdown(0, 0, images_size, &sidecars);

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
            tool_output_count: sidecars.tool_output_count,
            mcp_log_count: sidecars.mcp_log_count,
            history_lines: sidecars.history_lines,
            source_breakdown: breakdown,
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        fs::write(archive_dir.join("manifest.json"), manifest_json)?;

        println!("Archived session (clean) to: {}", archive_dir.display());
        println!("  Messages: {}", message_count);
        println!("  Images: {}", image_count);
        println!("  Size: {} KB", archive_size_bytes / 1024);
        println!("  conversation.md written");

        return Ok(archive_dir);
    }

    // Full mode (default): copy JSONL, copy agents, extract images.

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

    // Capture optional new sidecars (gated on the include set).
    // Status-quo callers leave all three flags off, keeping manifest
    // bytes identical to the pre-PR-2 layout.
    let sidecars = capture_optional_sidecars(
        &archive_dir,
        cwd_encoded.as_deref(),
        &session_id,
        window,
        include,
    );

    // Per-source byte counts for the v5 breakdown. session.jsonl bytes
    // are taken from the destination file (post image-extraction).
    let session_jsonl_bytes = fs::metadata(&dest_session).map(|m| m.len()).unwrap_or(0);
    let agents_bytes = total_agents_bytes(&archive_dir);
    let images_bytes: u64 = all_images.iter().map(|img| img.size_bytes).sum();
    let breakdown =
        build_source_breakdown(session_jsonl_bytes, agents_bytes, images_bytes, &sidecars);

    // Create manifest (schema v5).
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
        tool_output_count: sidecars.tool_output_count,
        mcp_log_count: sidecars.mcp_log_count,
        history_lines: sidecars.history_lines,
        source_breakdown: breakdown,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(archive_dir.join("manifest.json"), manifest_json)?;

    println!("Archived session to: {}", archive_dir.display());
    println!("  Messages: {}", message_count);
    println!("  Agents: {}", agents.len());
    println!("  Images: {}", image_count);
    println!("  Size: {} KB", size_bytes / 1024);

    Ok(archive_dir)
}

/// Bulk-archive every unarchived session under `~/.claude/projects/`.
pub(crate) fn save_all_sessions(
    clean: bool,
    include_agents_in_clean_md: bool,
    include: &IncludeSet,
) -> Result<ArchiveResult> {
    let projects_dir = crate::paths::claude_projects_dir();

    // A missing `~/.claude/projects/` is a normal state -- a fresh user
    // who has never run Claude, or a CI environment that has no live
    // Claude install, will land here. Treating it as a hard error makes
    // `mx codex archive --all` (and the `--archive-first` export hop
    // that wraps it) blow up on first run. Instead, we treat "no
    // projects dir" as "no sessions to archive", warn on stderr so the
    // operator knows nothing was scanned, and return an empty summary.
    if !projects_dir.exists() {
        eprintln!(
            "note: no Claude projects found at {}; nothing to archive",
            projects_dir.display()
        );
        return Ok(ArchiveResult::default());
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
    let mut summary = ArchiveResult::default();

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
                if archived_ids.contains(session_id) {
                    summary.skipped_count += 1;
                    continue;
                }

                println!("Archiving: {}", session_id);
                let dir = archive_session(&file_path, clean, include_agents_in_clean_md, include)?;
                summary.archive_paths.push(dir);
                summary.archived_count += 1;
            }
        }
    }

    println!("Archived {} new session(s)", summary.archived_count);

    Ok(summary)
}
