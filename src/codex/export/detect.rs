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
    /// True iff anything unarchived was found in either source
    /// (~/.claude/ or /tmp/claude-<uid>/).
    pub fn has_unarchived(&self) -> bool {
        self.unarchived_session_count > 0 || self.unarchived_tool_output_count > 0
    }

    /// Render the operator-facing warning text. Returns `None` when
    /// nothing is unarchived.
    ///
    /// S3: surfaces both `unarchived_session_count` (~/.claude/) and
    /// `unarchived_tool_output_count` (/tmp/claude-<uid>/) when nonzero.
    /// Either count alone is enough to print the warning; printing only
    /// the nonzero source keeps the noise focused.
    pub fn warning_text(&self) -> Option<String> {
        if !self.has_unarchived() {
            return None;
        }
        let head = match (
            self.unarchived_session_count,
            self.unarchived_tool_output_count,
        ) {
            (0, 0) => unreachable!("has_unarchived() guards this branch"),
            (sessions, 0) => format!(
                "note: {} unarchived session(s) detected in ~/.claude/.",
                sessions
            ),
            (0, tools) => format!(
                "note: {} session(s) with tool output in /tmp/ not yet archived.",
                tools
            ),
            (sessions, tools) => format!(
                "note: {} unarchived session(s) detected in ~/.claude/, \
                 and {} session(s) with tool output in /tmp/.",
                sessions, tools
            ),
        };
        let mut msg = head;
        msg.push_str(
            "\n       Run `mx codex save --all` to ingest, or rerun with --archive-first.",
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
///
/// **Test hook:** the env var `MX_CLAUDE_TMP_TASKS_DIR` overrides the
/// scan root entirely. Tests set it to an empty tempdir so the count is
/// deterministic regardless of what's actually in `/tmp/claude-<uid>/`
/// on the test runner. Setting it to `__SKIP__` short-circuits the scan
/// to zero (used by tests that want the legacy "sessions only" warning
/// shape without depending on disk state).
fn count_unarchived_tool_outputs(archived: &HashSet<String>) -> usize {
    let root: PathBuf = match std::env::var("MX_CLAUDE_TMP_TASKS_DIR") {
        Ok(override_path) => {
            if override_path == "__SKIP__" {
                return 0;
            }
            PathBuf::from(override_path)
        }
        Err(_) => {
            #[cfg(unix)]
            {
                // SAFETY: getuid(2) is infallible per POSIX.
                unsafe extern "C" {
                    fn getuid() -> u32;
                }
                let uid = unsafe { getuid() };
                PathBuf::from(format!("/tmp/claude-{}", uid))
            }
            #[cfg(not(unix))]
            {
                let _ = archived;
                return 0;
            }
        }
    };
    if !root.exists() {
        return 0;
    }
    count_in_tmp_root(&root, archived)
}

/// Inner walker for [`count_unarchived_tool_outputs`]. Pulled out so the
/// env-var override and the production /tmp branch share one
/// implementation.
fn count_in_tmp_root(root: &Path, archived: &HashSet<String>) -> usize {
    #[cfg(unix)]
    {
        if !root.exists() {
            return 0;
        }
        let mut count = 0usize;
        let user_dirs = match fs::read_dir(root) {
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
    use serial_test::serial;
    use std::sync::Mutex;

    // Process-wide lock so tests that twiddle MX_CLAUDE_TMP_TASKS_DIR
    // don't interleave with each other. Pairs with `#[serial]`.
    static TMP_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: sets `MX_CLAUDE_TMP_TASKS_DIR=__SKIP__` for the
    /// lifetime of the guard. Most detect-tests want a deterministic
    /// zero from the /tmp scan regardless of what's actually in
    /// `/tmp/claude-<uid>/` on the runner.
    struct SkipTmpScan {
        prev: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }
    impl SkipTmpScan {
        fn new() -> Self {
            let guard = TMP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("MX_CLAUDE_TMP_TASKS_DIR").ok();
            // SAFETY: env mutation guarded by TMP_ENV_LOCK + #[serial].
            unsafe {
                std::env::set_var("MX_CLAUDE_TMP_TASKS_DIR", "__SKIP__");
            }
            Self {
                prev,
                _guard: guard,
            }
        }
    }
    impl Drop for SkipTmpScan {
        fn drop(&mut self) {
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var("MX_CLAUDE_TMP_TASKS_DIR", v),
                    None => std::env::remove_var("MX_CLAUDE_TMP_TASKS_DIR"),
                }
            }
        }
    }

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
    #[serial]
    fn detect_zero_when_everything_archived() {
        let _skip = SkipTmpScan::new();
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
    #[serial]
    fn detect_some_unarchived() {
        let _skip = SkipTmpScan::new();
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
    #[serial]
    fn detect_many_unarchived_caps_sample_at_five() {
        let _skip = SkipTmpScan::new();
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
    #[serial]
    fn detect_skips_agent_files() {
        let _skip = SkipTmpScan::new();
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
    #[serial]
    fn detect_handles_missing_projects_dir() {
        let _skip = SkipTmpScan::new();
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("does-not-exist");
        let codex = tmp.path().join("codex");
        let report = detect_unarchived_in(&projects, &codex).unwrap();
        assert_eq!(report.unarchived_session_count, 0);
    }

    // ---- S3: warning surfaces tool-output count ----

    #[test]
    fn warning_text_combines_both_counts_when_both_nonzero() {
        let report = DetectionReport {
            unarchived_session_count: 23,
            unarchived_tool_output_count: 7,
            sample_unarchived_uuids: vec!["c3744b8d".to_string(), "a1b2c3d4".to_string()],
        };
        let warn = report.warning_text().expect("should produce warning");
        assert!(
            warn.contains("23 unarchived"),
            "missing session count: {warn}"
        );
        assert!(
            warn.contains("7 session(s) with tool output"),
            "missing tool-output count: {warn}"
        );
        assert!(warn.contains("/tmp/"), "should mention /tmp source: {warn}");
        assert!(
            warn.contains("mx codex save --all"),
            "should keep the remediation hint: {warn}"
        );
        assert!(
            warn.contains("c3744b8d"),
            "should keep sample uuids: {warn}"
        );
    }

    #[test]
    fn warning_text_tool_output_only_still_prints() {
        // /tmp/claude-<uid>/ has unarchived sessions but ~/.claude/ is
        // clean. Operator should still see the warning so the tool
        // outputs aren't lost.
        let report = DetectionReport {
            unarchived_session_count: 0,
            unarchived_tool_output_count: 3,
            sample_unarchived_uuids: vec![],
        };
        assert!(report.has_unarchived());
        let warn = report.warning_text().expect("tool-output-only must warn");
        assert!(warn.contains("3 session(s) with tool output"));
        assert!(!warn.contains("unarchived session(s) detected in ~/.claude/"));
    }

    #[test]
    #[serial]
    #[cfg(unix)]
    // Windows: count_in_tmp_root is cfg(unix)-gated (returns 0); the
    // override path is exercised only on unix. Tool-output capture is
    // a unix-only feature in v1 because /tmp/claude-<uid> has no
    // stable Windows analogue.
    fn detect_counts_unarchived_tool_outputs_via_override() {
        // Exercise the env-var override path: a fixture /tmp tree with
        // two sessions, one of which IS in the codex. Expect a count of
        // exactly 1 unarchived tool-output session.
        let guard = TMP_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let codex = tmp.path().join("codex");
        let tmp_root = tmp.path().join("tmp_claude");
        std::fs::create_dir_all(&projects).unwrap();

        // Codex has one archived session.
        write_manifest(&codex.join("2026-04-29-100000-aaaaaaaa"), "aaaaaaaa-1111");

        // /tmp fixture: two sessions, each with a `tasks/` dir.
        let user_slug = tmp_root.join("user-charlie");
        std::fs::create_dir_all(user_slug.join("aaaaaaaa-1111").join("tasks")).unwrap();
        std::fs::create_dir_all(user_slug.join("zzzzzzzz-9999").join("tasks")).unwrap();

        let prev = std::env::var("MX_CLAUDE_TMP_TASKS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CLAUDE_TMP_TASKS_DIR", &tmp_root);
        }
        let report = detect_unarchived_in(&projects, &codex).unwrap();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CLAUDE_TMP_TASKS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_TMP_TASKS_DIR"),
            }
        }
        drop(guard);

        // aaaaaaaa is archived → not counted; zzzzzzzz is not → counted.
        assert_eq!(report.unarchived_tool_output_count, 1);
        // Has-unarchived should fire on tool-output count alone.
        assert!(report.has_unarchived());
    }

    #[test]
    fn warning_text_sessions_only_omits_tool_output_phrase() {
        // Backwards-compatible shape when the /tmp source is clean.
        let report = DetectionReport {
            unarchived_session_count: 2,
            unarchived_tool_output_count: 0,
            sample_unarchived_uuids: vec!["aaaaaaaa".to_string()],
        };
        let warn = report.warning_text().unwrap();
        assert!(warn.contains("2 unarchived"));
        assert!(
            !warn.contains("tool output"),
            "no tool-output phrase when count is zero: {warn}"
        );
    }
}
