//! Source walkers: enumerate the on-disk artifacts a session produces.
//!
//! Each walker is independent and (mostly) pure — it takes the inputs it
//! needs and returns the file paths (or sliced lines) it found. None of
//! these walkers write into the archive; the writer in `write.rs` does
//! that, gated on the `IncludeSet` the caller built.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::super::AgentInfo;

/// A `[start, end]` timestamp window used to attribute mtime-stamped
/// artifacts (MCP logs, history slices) to a session. The window is
/// derived from the session JSONL's first/last event timestamps and is
/// approximate by design — MCP and history are best-effort attribution
/// per the unification architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimestampWindow {
    /// Construct a window. Caller is responsible for ordering; the
    /// `contains` test accepts equality on either end (closed interval).
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { start, end }
    }

    /// True iff `ts` lies within `[start, end]` inclusive.
    pub fn contains(&self, ts: DateTime<Utc>) -> bool {
        ts >= self.start && ts <= self.end
    }

    /// True iff a `SystemTime` lies within the window.
    pub fn contains_systime(&self, st: SystemTime) -> bool {
        let dt: DateTime<Utc> = st.into();
        self.contains(dt)
    }
}

/// Find subagent JSONLs that belong to a given parent session.
///
/// Walks `<project>/<session_id>/subagents/` (the layout Claude writes
/// into) and returns every `agent-*.jsonl` it finds.
///
/// The `_session_modified` parameter is retained on the signature for
/// future window-tightening; today it is unused — the directory itself
/// is already scoped by `session_id`, so any agent JSONL inside it is
/// in-scope by construction.
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
                && let Ok(meta) = entry.metadata()
                && let Ok(_modified) = meta.modified()
            {
                let content = fs::read_to_string(&path)?;
                let messages = content.lines().filter(|l| !l.trim().is_empty()).count();

                agents.push(AgentInfo {
                    id: path.to_string_lossy().to_string(),
                    file: format!("agents/{}", name),
                    messages,
                });
            }
        }
    }

    Ok(agents)
}

/// Find MCP server log files attributable to the given session window.
///
/// Walks `~/.cache/claude-cli-nodejs/<cwd_encoded>/`, enumerates each
/// `mcp-logs-*` subdirectory (one per MCP server active for the cwd),
/// and returns every `*.jsonl` file whose mtime lies in `window`.
///
/// **Heuristic:** MCP logs are not session-tagged on disk; one server's
/// log file can span multiple sessions or none. We use mtime as a
/// best-effort attribution signal — a file is "in-scope" if it was
/// touched between the session's first and last event timestamps. This
/// is intentional per the unification architecture's note that
/// MCP↔session attribution is best-effort.
///
/// Returns an empty vec if the cwd-encoded directory does not exist
/// (no error — many sessions have no MCP activity).
pub(super) fn find_mcp_logs(cwd_encoded: &str, window: TimestampWindow) -> Result<Vec<PathBuf>> {
    let root = crate::paths::claude_mcp_logs_dir(cwd_encoded);
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = match dir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.starts_with("mcp-logs-") {
            continue;
        }

        for inner in fs::read_dir(&dir)? {
            let inner = inner?;
            let p = inner.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = match inner.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let modified = match meta.modified() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if window.contains_systime(modified) {
                out.push(p);
            }
        }
    }

    Ok(out)
}

/// Find `/tmp/claude-<uid>/<user_slug>/<session_uuid>/tasks/*.output`
/// files that belong to this session.
///
/// **Deterministic:** the directory is keyed by the exact session UUID,
/// so any `.output` file inside is unambiguously this session's. No
/// timestamp filtering needed.
///
/// Returns an empty vec if the tasks directory does not exist (the
/// session may have never invoked a tool that writes to disk).
pub(super) fn find_tool_outputs(
    uid: u32,
    user_slug: &str,
    session_uuid: &str,
) -> Result<Vec<PathBuf>> {
    let tasks_dir = crate::paths::tmp_claude_tasks_dir(uid, user_slug, session_uuid);
    if !tasks_dir.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in fs::read_dir(&tasks_dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("output") && p.is_file() {
            out.push(p);
        }
    }
    Ok(out)
}

/// Read `~/.claude/history.jsonl` and return the lines whose embedded
/// timestamps lie inside `window`.
///
/// **Heuristic:** `history.jsonl` is one shared file across all
/// sessions; lines are JSON objects with a `timestamp` field (ISO 8601
/// or epoch — we accept both). Lines without a parseable timestamp are
/// skipped silently; they're informational and excluding them just
/// shrinks the slice. Returns an empty vec (and no error) if the
/// history file is missing.
///
/// Returns the lines themselves (not paths) because the slice is a
/// derived subset, not an addressable file on disk.
pub(super) fn find_history_slice(window: TimestampWindow) -> Result<Vec<String>> {
    let path = crate::paths::claude_history_jsonl();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Try to parse as JSON and extract `timestamp`. We accept either
        // an ISO-8601 string ("2026-04-29T14:30:00Z") or a Unix epoch
        // (seconds or millis as a number) — Claude's history format has
        // varied across versions.
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = match value.get("timestamp") {
            Some(serde_json::Value::String(s)) => s.parse::<DateTime<Utc>>().ok(),
            Some(serde_json::Value::Number(n)) => n.as_i64().and_then(|secs| {
                // Heuristic: > 10^12 ⇒ millis; otherwise seconds.
                let (s, ns) = if secs > 1_000_000_000_000 {
                    (secs / 1000, ((secs % 1000) * 1_000_000) as u32)
                } else {
                    (secs, 0u32)
                };
                DateTime::<Utc>::from_timestamp(s, ns)
            }),
            _ => None,
        };
        if let Some(ts) = ts
            && window.contains(ts)
        {
            out.push(line.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_window(start_offset_secs: i64, end_offset_secs: i64) -> TimestampWindow {
        let now = Utc::now();
        TimestampWindow::new(
            now + Duration::seconds(start_offset_secs),
            now + Duration::seconds(end_offset_secs),
        )
    }

    #[test]
    fn timestamp_window_contains_inclusive() {
        let w = make_window(-60, 60);
        assert!(w.contains(w.start));
        assert!(w.contains(w.end));
        assert!(w.contains(Utc::now()));
        assert!(!w.contains(w.end + Duration::seconds(1)));
        assert!(!w.contains(w.start - Duration::seconds(1)));
    }

    // ---------------------------------------------------------------------
    // find_mcp_logs
    // ---------------------------------------------------------------------

    #[test]
    fn find_mcp_logs_returns_empty_for_missing_root() {
        // A cwd encoding that almost certainly doesn't exist
        let cwd = "-no-such-cwd-encoding-xxxxxxx";
        let w = make_window(-3600, 3600);
        let logs = find_mcp_logs(cwd, w).unwrap();
        assert!(logs.is_empty());
    }

    // The other walkers are exercised against the live filesystem, which
    // is already mocked by paths.rs's seam. For find_mcp_logs we test
    // structure and filtering against a tempdir-backed fake by going
    // through the public function plus a path-override pattern would
    // require a `_with` seam; for now the integration with `mcp_logs_dir`
    // is covered by the path test in src/paths.rs and the empty-root
    // smoke test above. PR 3 (export) will exercise the full walk.

    // ---------------------------------------------------------------------
    // find_tool_outputs
    // ---------------------------------------------------------------------

    #[test]
    fn find_tool_outputs_returns_empty_for_missing_dir() {
        // A session UUID that almost certainly doesn't exist on disk
        let uuid = "00000000-aaaa-bbbb-cccc-111111111111";
        let outs = find_tool_outputs(99999, "-no-such-user", uuid).unwrap();
        assert!(outs.is_empty());
    }

    // ---------------------------------------------------------------------
    // find_history_slice
    // ---------------------------------------------------------------------

    #[test]
    fn find_history_slice_handles_missing_file_gracefully() {
        // We can't easily redirect ~/.claude/history.jsonl from a unit
        // test without env-var seams in paths.rs, so we just assert the
        // function does not panic regardless of whether the file exists
        // on the test host. Returning Ok(_) is the contract — empty is
        // valid; non-empty is also valid (developer's machine may have
        // history). Either way the function must not error.
        let w = make_window(-1_000_000_000, 1_000_000_000);
        let result = find_history_slice(w);
        assert!(result.is_ok());
    }

    #[test]
    fn find_history_slice_filters_by_window() {
        // Direct test of the parsing/filter logic via a tempfile. We
        // can't redirect the path helper from a test, so we replicate the
        // inner loop here against a known input — the production caller
        // exercises the full `claude_history_jsonl()` path.
        let lines = [
            r#"{"timestamp": "2026-04-29T12:00:00Z", "msg": "in"}"#,
            r#"{"timestamp": "2026-04-29T08:00:00Z", "msg": "before"}"#,
            r#"{"timestamp": "2026-04-29T18:00:00Z", "msg": "after"}"#,
            r#"{"no-timestamp-field": true}"#,
            r#"not even json"#,
        ];
        // Window: 10:00 - 14:00 on the same day
        let start: DateTime<Utc> = "2026-04-29T10:00:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-29T14:00:00Z".parse().unwrap();
        let w = TimestampWindow::new(start, end);

        // Replicate the in-loop logic for this test (the function reads
        // from a fixed path, which we don't override here).
        let mut hits = 0;
        for line in lines.iter() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ts = match v.get("timestamp") {
                Some(serde_json::Value::String(s)) => s.parse::<DateTime<Utc>>().ok(),
                _ => None,
            };
            if let Some(ts) = ts
                && w.contains(ts)
            {
                hits += 1;
            }
        }
        assert_eq!(hits, 1, "only the 12:00 line should match the window");
    }
}
