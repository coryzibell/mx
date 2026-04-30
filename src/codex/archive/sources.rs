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

/// Derive the session's timestamp window from its JSONL.
///
/// Walks the session JSONL once, parsing each line for a top-level
/// `timestamp` (ISO-8601 string). Returns:
///
/// - `session_start` = first parseable timestamp in the file
/// - `session_end`   = last parseable timestamp in the file
///
/// Falls back to file metadata if the JSONL is empty or has no
/// parseable timestamps:
///
/// - `session_end`   <- file mtime
/// - `session_start` <- file `created()` (or mtime if `created()` is
///   unsupported on the filesystem)
///
/// On the fallback path a `tracing`-style stderr warning is emitted so
/// the operator knows the heuristic kicked in. Empty / unparseable
/// histories are not an error — the unification architecture lets the
/// session window be approximate.
pub(super) fn derive_session_window(session_path: &Path) -> Result<TimestampWindow> {
    let metadata = fs::metadata(session_path)
        .with_context(|| format!("session metadata: {}", session_path.display()))?;

    let mut first: Option<DateTime<Utc>> = None;
    let mut last: Option<DateTime<Utc>> = None;

    if let Ok(content) = fs::read_to_string(session_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ts = match value.get("timestamp") {
                Some(serde_json::Value::String(s)) => s.parse::<DateTime<Utc>>().ok(),
                Some(serde_json::Value::Number(n)) => n.as_i64().and_then(|secs| {
                    let (s, ns) = if secs > 1_000_000_000_000 {
                        (secs / 1000, ((secs % 1000) * 1_000_000) as u32)
                    } else {
                        (secs, 0u32)
                    };
                    DateTime::<Utc>::from_timestamp(s, ns)
                }),
                _ => None,
            };
            if let Some(ts) = ts {
                if first.is_none() {
                    first = Some(ts);
                }
                last = Some(ts);
            }
        }
    }

    if let (Some(s), Some(e)) = (first, last) {
        // Order defensively: a malformed JSONL with shuffled timestamps
        // shouldn't yield an inverted window.
        let (start, end) = if s <= e { (s, e) } else { (e, s) };
        return Ok(TimestampWindow::new(start, end));
    }

    // Fallback: derive from file metadata. mtime is the most reliable
    // proxy for "when was the session last touched"; created() can be
    // unavailable on some filesystems, so we degrade to mtime.
    let mtime: DateTime<Utc> = metadata.modified()?.into();
    let created: DateTime<Utc> = metadata.created().map(|c| c.into()).unwrap_or(mtime);
    let (start, end) = if created <= mtime {
        (created, mtime)
    } else {
        (mtime, created)
    };
    eprintln!(
        "warning: no parseable timestamps in {} — falling back to file metadata for session window",
        session_path.display()
    );
    Ok(TimestampWindow::new(start, end))
}

/// Find subagent JSONLs that belong to a given parent session.
///
/// Walks `<project>/<session_id>/subagents/` (the layout Claude writes
/// into) and returns every `agent-*.jsonl` it finds.
///
/// The directory itself is already scoped by `session_id`, so any agent
/// JSONL inside it is in-scope by construction. No timestamp filtering
/// is needed here — that's why this walker takes no `TimestampWindow`.
pub(super) fn find_agent_sessions(session_path: &Path) -> Result<Vec<AgentInfo>> {
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
    Ok(filter_history_lines(&content, &window))
}

/// Pure: pick the lines from a `history.jsonl`-shaped string whose
/// embedded `timestamp` lies within `window`.
///
/// Extracted out of `find_history_slice` so production AND tests
/// exercise the same code (W4 from Verdictia's PR #268 review). Lines
/// without a parseable JSON object, or without a parseable `timestamp`
/// field, are dropped silently — they were not attributable to the
/// session window and Claude's history format has varied enough across
/// versions that we don't want to error on a malformed line.
pub(super) fn filter_history_lines(content: &str, window: &TimestampWindow) -> Vec<String> {
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
    out
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

    // ---------------------------------------------------------------------
    // derive_session_window
    // ---------------------------------------------------------------------

    #[test]
    fn derive_session_window_uses_jsonl_first_and_last_timestamps() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("c3744b8d.jsonl");
        let body = concat!(
            r#"{"role":"user","timestamp":"2026-04-29T10:00:00Z"}"#,
            "\n",
            r#"{"role":"assistant","timestamp":"2026-04-29T10:05:00Z"}"#,
            "\n",
            r#"{"role":"user","timestamp":"2026-04-29T10:42:30Z"}"#,
            "\n",
        );
        fs::write(&session_path, body).unwrap();

        let w = derive_session_window(&session_path).unwrap();
        let expected_start: DateTime<Utc> = "2026-04-29T10:00:00Z".parse().unwrap();
        let expected_end: DateTime<Utc> = "2026-04-29T10:42:30Z".parse().unwrap();
        assert_eq!(w.start, expected_start);
        assert_eq!(w.end, expected_end);
    }

    #[test]
    fn derive_session_window_skips_lines_without_parseable_timestamps() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("aa.jsonl");
        let body = concat!(
            "not json\n",
            r#"{"no":"timestamp"}"#,
            "\n",
            r#"{"role":"user","timestamp":"2026-04-29T11:00:00Z"}"#,
            "\n",
            r#"{"role":"assistant","timestamp":"2026-04-29T11:30:00Z"}"#,
            "\n",
        );
        fs::write(&session_path, body).unwrap();

        let w = derive_session_window(&session_path).unwrap();
        let expected_start: DateTime<Utc> = "2026-04-29T11:00:00Z".parse().unwrap();
        let expected_end: DateTime<Utc> = "2026-04-29T11:30:00Z".parse().unwrap();
        assert_eq!(w.start, expected_start);
        assert_eq!(w.end, expected_end);
    }

    #[test]
    fn derive_session_window_falls_back_to_metadata_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("empty.jsonl");
        fs::write(&session_path, "").unwrap();

        // The fallback path uses file metadata: end <- mtime, start <- created (or mtime).
        // Sanity: the call must succeed and produce a window that contains
        // `now` within a reasonable slop.
        let w = derive_session_window(&session_path).unwrap();
        let now = Utc::now();
        // start <= end is required; either end is "right now" or close to it.
        assert!(w.start <= w.end, "fallback window must be ordered");
        let drift = (now - w.end).num_seconds().abs();
        assert!(drift < 60, "fallback end should be close to mtime/now");
    }

    #[test]
    fn derive_session_window_falls_back_when_no_parseable_timestamps() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("garbage.jsonl");
        fs::write(&session_path, "not json\n{\"no\":1}\n").unwrap();
        let w = derive_session_window(&session_path).unwrap();
        // We can't assert exact values, but the call must succeed and
        // return a non-inverted window.
        assert!(w.start <= w.end);
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
    fn filter_history_lines_keeps_only_in_window() {
        // W4: this hits the SAME `filter_history_lines` function the
        // production walker calls — the previous version of this test
        // was a copy of the inner loop, which insulated the test from
        // any real regression in the production code path.
        let content = concat!(
            r#"{"timestamp": "2026-04-29T12:00:00Z", "msg": "in"}"#,
            "\n",
            r#"{"timestamp": "2026-04-29T08:00:00Z", "msg": "before"}"#,
            "\n",
            r#"{"timestamp": "2026-04-29T18:00:00Z", "msg": "after"}"#,
            "\n",
            r#"{"no-timestamp-field": true}"#,
            "\n",
            "not even json\n",
            "\n", // blank line — should be skipped
        );
        let start: DateTime<Utc> = "2026-04-29T10:00:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-29T14:00:00Z".parse().unwrap();
        let w = TimestampWindow::new(start, end);

        let kept = filter_history_lines(content, &w);
        assert_eq!(kept.len(), 1, "expected exactly one in-window line");
        assert!(kept[0].contains("\"in\""));
    }

    #[test]
    fn filter_history_lines_accepts_epoch_seconds_and_millis() {
        let secs: i64 = 1_777_809_600; // 2026-05-04T00:00:00Z
        let millis: i64 = secs * 1_000 + 250;
        let content = format!(
            "{{\"timestamp\": {secs}, \"msg\": \"sec\"}}\n{{\"timestamp\": {millis}, \"msg\": \"ms\"}}\n",
        );
        let start = DateTime::<Utc>::from_timestamp(secs - 60, 0).unwrap();
        let end = DateTime::<Utc>::from_timestamp(secs + 60, 0).unwrap();
        let w = TimestampWindow::new(start, end);
        let kept = filter_history_lines(&content, &w);
        assert_eq!(kept.len(), 2, "both seconds and millis variants must match");
    }
}
