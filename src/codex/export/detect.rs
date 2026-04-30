//! Detect live Claude data that has not yet been archived into the codex.
//!
//! Runs at the start of `mx codex export`. Two scans:
//!
//! 1. `~/.claude/projects/<project-slug>/<session-uuid>.jsonl` — every
//!    session JSONL Claude has on disk.
//! 2. `/tmp/claude-<uid>/<user-slug>/<session-uuid>/tasks/` — per-uid
//!    scratch with tool outputs.
//!
//! Each session UUID is checked against the codex by walking
//! `<codex_dir>/<archive_dir>/manifest.json` once and building a set of
//! archived `session_id`s. The detection report counts unarchived
//! sessions in each source, plus a few sample UUIDs for the warning.
//!
//! **Important:** this module reads `~/.claude/` ONLY for detection — it
//! never reads session content for rendering. Export's content path goes
//! through the codex archive directory exclusively.

use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Maximum sample UUIDs to include in the warning. Keeps stderr noise
/// bounded even when hundreds of sessions are unarchived.
const SAMPLE_CAP: usize = 5;

/// Result of scanning the live Claude data sources for unarchived
/// sessions.
#[derive(Debug, Clone, Default)]
pub struct DetectionReport {
    /// Sessions present under `~/.claude/projects/` but not in the codex.
    pub unarchived_session_count: usize,
    /// Sessions whose `/tmp/claude-<uid>/.../tasks/` dir exists but the
    /// session itself has no codex manifest. Subset signal that's
    /// useful when the user just stopped a tool-using session.
    pub unarchived_tool_output_count: usize,
    /// Up to `SAMPLE_CAP` short UUIDs (first 8 chars) for the warning.
    pub sample_unarchived_uuids: Vec<String>,
}

impl DetectionReport {
    /// True iff anything unarchived was found.
    pub fn has_unarchived(&self) -> bool {
        self.unarchived_session_count > 0
    }

    /// Render the operator-facing warning text. Returns `None` when
    /// nothing is unarchived.
    pub fn warning_text(&self) -> Option<String> {
        if !self.has_unarchived() {
            return None;
        }
        let mut msg = format!(
            "note: {} unarchived session(s) detected in ~/.claude/. \
             Run `mx codex archive --all` to ingest, or rerun with --archive-first.",
            self.unarchived_session_count
        );
        if !self.sample_unarchived_uuids.is_empty() {
            msg.push_str("\n       Sample: ");
            msg.push_str(&self.sample_unarchived_uuids.join(", "));
        }
        Some(msg)
    }
}

/// Override hook for tests: redirect the `~/.claude/projects` scan to a
/// custom directory. Production callers just use `detect_unarchived()`.
pub fn detect_unarchived() -> Result<DetectionReport> {
    let projects_dir = match std::env::var("MX_CLAUDE_PROJECTS_DIR") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => crate::paths::claude_projects_dir(),
    };
    detect_unarchived_in(&projects_dir, &crate::paths::codex_dir())
}

/// Pure: scan `projects_dir` and `codex_dir` and report unarchived sessions.
///
/// Extracted so unit tests can run against tempdirs without process-wide
/// env mutation. The /tmp tasks scan is always done against the live
/// `/tmp/claude-<uid>/...` tree because it's keyed off the running uid;
/// for unit testing we keep that scan separate (see
/// `count_unarchived_tool_outputs`).
pub fn detect_unarchived_in(projects_dir: &Path, codex_dir: &Path) -> Result<DetectionReport> {
    let archived = collect_archived_session_ids(codex_dir)?;
    let mut report = DetectionReport::default();
    let mut samples: Vec<String> = Vec::new();

    if projects_dir.exists() {
        for proj in fs::read_dir(projects_dir)? {
            let proj = proj?;
            let proj_dir = proj.path();
            if !proj_dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&proj_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s,
                    None => continue,
                };
                // Skip agent files: those mirror their parent session and
                // aren't independently archivable.
                if stem.starts_with("agent-") {
                    continue;
                }
                if archived.contains(stem) {
                    continue;
                }
                report.unarchived_session_count += 1;
                if samples.len() < SAMPLE_CAP {
                    let short = stem.chars().take(8).collect::<String>();
                    samples.push(short);
                }
            }
        }
    }

    report.sample_unarchived_uuids = samples;
    report.unarchived_tool_output_count = count_unarchived_tool_outputs(&archived);
    Ok(report)
}

/// Count session UUIDs under `/tmp/claude-<uid>/<user_slug>/` that don't
/// have a matching codex manifest. Best-effort — silently returns 0 if
/// the tmp tree is missing or unreadable.
fn count_unarchived_tool_outputs(archived: &HashSet<String>) -> usize {
    // The tmp layout encodes uid + user_slug at build time, so we walk
    // every <session_uuid>/tasks under any user-slug subdirectory we
    // can find. For non-Unix targets, return 0.
    #[cfg(unix)]
    {
        // SAFETY: getuid(2) is infallible per POSIX.
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        let uid = unsafe { getuid() };
        let root = PathBuf::from(format!("/tmp/claude-{}", uid));
        if !root.exists() {
            return 0;
        }
        let mut count = 0usize;
        let user_dirs = match fs::read_dir(&root) {
            Ok(rd) => rd,
            Err(_) => return 0,
        };
        for user_entry in user_dirs.flatten() {
            let user_dir = user_entry.path();
            if !user_dir.is_dir() {
                continue;
            }
            let session_dirs = match fs::read_dir(&user_dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for sess_entry in session_dirs.flatten() {
                let sess_dir = sess_entry.path();
                let session_uuid = match sess_dir.file_name().and_then(|n| n.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let tasks_dir = sess_dir.join("tasks");
                if !tasks_dir.exists() {
                    continue;
                }
                if !archived.contains(&session_uuid) {
                    count += 1;
                }
            }
        }
        count
    }
    #[cfg(not(unix))]
    {
        let _ = archived;
        0
    }
}

/// Walk the codex directory once and return every session_id present in
/// a manifest. Skips the `by-project*` accessory dirs.
fn collect_archived_session_ids(codex_dir: &Path) -> Result<HashSet<String>> {
    let mut ids = HashSet::new();
    if !codex_dir.exists() {
        return Ok(ids);
    }
    for entry in fs::read_dir(codex_dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if matches!(name, "by-project" | "by-project.staging" | "by-project.old") {
            continue;
        }
        let manifest_path = p.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let raw = match fs::read_to_string(&manifest_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let manifest: crate::codex::Manifest = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(_) => continue,
        };
        ids.insert(manifest.session_id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn write_manifest(archive_dir: &Path, session_id: &str) {
        fs::create_dir_all(archive_dir).unwrap();
        let manifest = crate::codex::Manifest {
            version: crate::codex::MANIFEST_WRITE_VERSION,
            session_id: session_id.to_string(),
            archived_at: Utc::now(),
            session_start: Utc::now(),
            session_end: Utc::now(),
            project_path: Some("/home/test/proj".to_string()),
            message_count: 0,
            agent_count: 0,
            agents: vec![],
            size_bytes: 0,
            checksum: "sha256:zero".to_string(),
            image_count: None,
            images: None,
            has_clean_transcript: None,
            user_name: None,
            assistant_name: None,
            tool_output_count: None,
            mcp_log_count: None,
            history_lines: None,
            source_breakdown: None,
        };
        fs::write(
            archive_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_session_jsonl(projects_dir: &Path, project_slug: &str, session_uuid: &str) {
        let dir = projects_dir.join(project_slug);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", session_uuid));
        fs::write(&path, "{}\n").unwrap();
    }

    #[test]
    fn detect_zero_when_everything_archived() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let codex = tmp.path().join("codex");
        write_session_jsonl(&projects, "-home-charlie-mx", "aaaaaaaa-1111");
        write_manifest(&codex.join("2026-04-29-100000-aaaaaaaa"), "aaaaaaaa-1111");

        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 0);
        assert!(!report.has_unarchived());
        assert!(report.warning_text().is_none());
    }

    #[test]
    fn detect_some_unarchived() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let codex = tmp.path().join("codex");
        write_session_jsonl(&projects, "-home-charlie-mx", "aaaaaaaa-1111");
        write_session_jsonl(&projects, "-home-charlie-mx", "bbbbbbbb-2222");
        write_session_jsonl(&projects, "-home-charlie-wonka", "cccccccc-3333");
        // Only one of three is archived.
        write_manifest(&codex.join("2026-04-29-100000-aaaaaaaa"), "aaaaaaaa-1111");

        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 2);
        assert!(report.has_unarchived());
        assert_eq!(report.sample_unarchived_uuids.len(), 2);
        let warn = report.warning_text().unwrap();
        assert!(warn.contains("2 unarchived"), "got: {warn}");
    }

    #[test]
    fn detect_many_unarchived_caps_sample_at_five() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let codex = tmp.path().join("codex");
        for i in 0..12 {
            write_session_jsonl(
                &projects,
                "-home-charlie-mx",
                &format!("{:08x}-1111", i + 1),
            );
        }
        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 12);
        assert_eq!(report.sample_unarchived_uuids.len(), SAMPLE_CAP);
    }

    #[test]
    fn detect_skips_agent_files() {
        // agent-*.jsonl mirrors the parent and is not independently
        // archivable — must not count toward unarchived_session_count.
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let codex = tmp.path().join("codex");
        let dir = projects.join("-home-charlie-mx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("agent-1234567890ab.jsonl"), "{}\n").unwrap();

        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 0);
    }

    #[test]
    fn detect_handles_missing_projects_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("does-not-exist");
        let codex = tmp.path().join("codex");
        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 0);
    }
}
