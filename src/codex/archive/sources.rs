//! Source walkers: enumerate the on-disk artifacts a session produces.
//!
//! Each walker is independent and pure(ish) — it takes the inputs it
//! needs and returns the file paths (or sliced lines) it found. None of
//! these walkers write into the archive; the writer in `write.rs` does
//! that, gated on the `IncludeSet` the caller built.
//!
//! In commit 1 of PR 2 only the existing subagent walker lives here
//! (consolidated from the historical `find_agent_sessions`). Later
//! commits add `find_mcp_logs`, `find_tool_outputs`, `find_history_slice`.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use super::super::AgentInfo;

/// Find subagent JSONLs that belong to a given parent session.
///
/// Walks `<project>/<session_id>/subagents/` (the layout Claude writes
/// into) and returns every `agent-*.jsonl` it finds. The historical
/// implementation used a per-file mtime guard but accepted everything
/// in the directory; this consolidated version preserves that behavior
/// because the directory is itself scoped by `session_id`, so the mtime
/// check was effectively unconditional.
///
/// The `_session_modified` parameter is retained on the signature for
/// future window-tightening; today it is unused.
pub(super) fn find_agent_sessions(
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
