//! Vault-backfill walker.
//!
//! Walks `~/.wonka/vault/archives/` (or a caller-supplied path) and feeds
//! every session JSONL it finds back through `archive::run` so the codex
//! ingests historical clean-shift snapshots.
//!
//! Layout we walk (verified empirically against the live vault — every
//! observed snapshot has the same shape):
//!
//! ```text
//! <vault_path>/
//!   session-YYYYMMDD-HHMMSS-NNNNNN/
//!     projects/
//!       <project-slug>/
//!         <session-uuid>.jsonl
//!         <session-uuid>/
//!           subagents/
//!             agent-*.jsonl
//!     plans/         (optional, often empty)
//!     history.jsonl  (slash-command history slice)
//! ```
//!
//! For each session JSONL we delegate to
//! `archive::run(ArchiveRequest::Single, options)` — the existing
//! pipeline already dedups against `manifest.session_id` via the
//! `archived_ids` mechanism in `write::save_all_sessions`, but `Single`
//! has no such guard, so we maintain our own seen-set here. (PR 4 left
//! `Single` archive intentionally unconditional; backfill is the first
//! caller that actually needs idempotence over the same source set.)
//!
//! Per-session failures are non-fatal: the bulk operation must survive a
//! single corrupt JSONL. Errors are accumulated on the report.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::Manifest;
use super::{ArchiveOptions, ArchiveRequest, get_codex_dir, run};

/// Aggregated outcome of a `--backfill` run.
#[derive(Debug, Default)]
pub struct BackfillReport {
    /// The vault root we walked.
    pub vault_path: PathBuf,
    /// How many `session-*` snapshot directories we visited.
    pub vault_snapshots_walked: usize,
    /// How many session JSONLs we found across all snapshots.
    pub sessions_found: usize,
    /// How many were freshly archived into the codex.
    pub sessions_archived: usize,
    /// How many were skipped because their `session_id` was already in
    /// the codex (dedup hit).
    pub sessions_skipped_already_archived: usize,
    /// Per-session failures (non-fatal). Reported at the end of the run.
    pub errors: Vec<(PathBuf, anyhow::Error)>,
}

/// Walk `vault_path` and feed every session JSONL back through the
/// archive pipeline. See module docs for layout.
pub fn run_backfill(vault_path: &Path, options: ArchiveOptions) -> Result<BackfillReport> {
    let mut report = BackfillReport {
        vault_path: vault_path.to_path_buf(),
        ..Default::default()
    };

    if !vault_path.exists() {
        anyhow::bail!(
            "vault path does not exist: {}. The vault may have already been removed.",
            vault_path.display()
        );
    }

    // Collect the vault snapshot directories first so we can emit
    // accurate progress ("12/26") and so `read_dir` ordering doesn't
    // matter to the final report.
    let snapshots = collect_snapshot_dirs(vault_path)
        .with_context(|| format!("walking vault root {}", vault_path.display()))?;

    if snapshots.is_empty() {
        eprintln!(
            "warning: vault path {} has no session-* snapshots; nothing to backfill",
            vault_path.display()
        );
        return Ok(report);
    }

    // Pre-compute the set of already-archived session_ids. This is the
    // same trick `save_all_sessions` uses; we replicate it here so the
    // `Single` codepath gets dedup too. Failures to read the codex dir
    // are non-fatal — we'd rather over-archive (and let the suffix
    // collision logic in `determine_archive_dir` handle it) than abort
    // the whole run. But we MUST surface the failure: silently swallowing
    // it (W1 from the PR 272 review) means a permission-denied or
    // corrupted manifest produces zero dedup, which leads to
    // double-archiving on re-run with no warning to the operator.
    let mut seen: HashSet<String> = match collect_archived_ids() {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!(
                "warning: failed to scan codex for dedup set: {e}; \
                 backfill may produce duplicates"
            );
            let codex_dir = get_codex_dir().unwrap_or_else(|_| PathBuf::from("<codex>"));
            report.errors.push((
                codex_dir,
                e.context("scanning codex for already-archived session_ids (dedup set)"),
            ));
            HashSet::new()
        }
    };

    let total_snapshots = snapshots.len();
    for (idx, snapshot) in snapshots.iter().enumerate() {
        report.vault_snapshots_walked += 1;

        let sessions = match collect_session_jsonls(snapshot) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warning: failed to walk vault snapshot {}: {}",
                    snapshot.display(),
                    e
                );
                report.errors.push((snapshot.clone(), e));
                continue;
            }
        };

        for session_path in sessions {
            report.sessions_found += 1;

            // Dedup: derive session_id from the filename
            // (`<uuid>.jsonl` → `<uuid>`) and skip if we've already
            // archived it. The session uuid IS the canonical id used by
            // the archive pipeline, so this matches what `archive::run`
            // would compute internally.
            let session_id = match session_id_from_path(&session_path) {
                Some(id) => id,
                None => {
                    let err = anyhow::anyhow!(
                        "could not derive session_id from {}",
                        session_path.display()
                    );
                    report.errors.push((session_path, err));
                    continue;
                }
            };

            if seen.contains(&session_id) {
                report.sessions_skipped_already_archived += 1;
                continue;
            }

            match run(
                ArchiveRequest::Single(session_path.clone()),
                options.clone(),
            ) {
                Ok(_result) => {
                    report.sessions_archived += 1;
                    seen.insert(session_id);
                }
                Err(e) => {
                    eprintln!(
                        "warning: failed to archive {}: {}",
                        session_path.display(),
                        e
                    );
                    report.errors.push((session_path, e));
                }
            }
        }

        eprintln!(
            "Backfilling vault: {}/{} snapshots, {}/{} sessions archived...",
            idx + 1,
            total_snapshots,
            report.sessions_archived,
            report.sessions_found
        );
    }

    eprintln!(
        "Backfill complete: {} vault snapshots, {} sessions found, {} archived, \
         {} already in codex, {} errors.",
        report.vault_snapshots_walked,
        report.sessions_found,
        report.sessions_archived,
        report.sessions_skipped_already_archived,
        report.errors.len()
    );

    Ok(report)
}

/// Enumerate `session-*` subdirectories under `vault_path`. Non-matching
/// entries (e.g. stray files) are silently skipped — the vault is a
/// best-effort historical store, not a sealed contract.
fn collect_snapshot_dirs(vault_path: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(vault_path)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match path.file_name().and_then(|n| n.to_str()) {
            Some(name) if name.starts_with("session-") => out.push(path),
            _ => continue,
        }
    }
    out.sort();
    Ok(out)
}

/// Walk `<snapshot>/projects/<slug>/*.jsonl` and return every session
/// JSONL we find. Subagent files (which live one directory deeper in
/// `<uuid>/subagents/`) are deliberately NOT returned — `archive::run`
/// finds them automatically via `find_agent_sessions` so we'd just
/// re-archive them as if they were primary sessions.
fn collect_session_jsonls(snapshot_dir: &Path) -> Result<Vec<PathBuf>> {
    let projects = snapshot_dir.join("projects");
    if !projects.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for slug_entry in fs::read_dir(&projects)? {
        let slug_entry = slug_entry?;
        let slug_path = slug_entry.path();
        if !slug_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&slug_path)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            // Skip stray agent-*.jsonl that may have been deposited at
            // the slug root (shouldn't happen per the vault layout but
            // defend anyway — the agent walker handles them).
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with("agent-")
            {
                continue;
            }
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Build the set of session_ids already present in the codex. Mirrors
/// the inline logic in `write::save_all_sessions` — could be hoisted to
/// a shared helper later, but two call sites isn't enough churn yet.
fn collect_archived_ids() -> Result<HashSet<String>> {
    let codex_dir = get_codex_dir()?;
    let mut ids = HashSet::new();
    if !codex_dir.exists() {
        return Ok(ids);
    }
    for entry in fs::read_dir(&codex_dir)? {
        let entry = entry?;
        let manifest_path = entry.path().join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let content = match fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let manifest: Manifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => continue,
        };
        ids.insert(manifest.session_id);
    }
    Ok(ids)
}

/// Derive the dedup key for a vault session JSONL from its filename
/// stem (`<uuid>.jsonl` → `<uuid>`).
///
/// This mirrors what `archive::run(ArchiveRequest::Single, ...)` ends
/// up using as the canonical `session_id` (the manifest's `session_id`
/// is currently set from the same file stem). If that ever changes —
/// e.g., the canonical id starts coming from a `sessionId` field
/// inside the JSONL — this function and the corresponding NOTE in
/// `archive::run` (`mod.rs`, the `Single` arm) must be updated
/// together. See W2 in PR 272 review for context.
fn session_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Build a minimal vault layout under `root` containing `n_snapshots`
    /// snapshots, each with one project slug and `sessions_per_snapshot`
    /// session JSONLs. Returns the vault path.
    fn build_fake_vault(root: &Path, n_snapshots: usize, sessions_per_snapshot: usize) -> PathBuf {
        let vault = root.join("vault-archives");
        fs::create_dir_all(&vault).unwrap();
        for s in 0..n_snapshots {
            let snap = vault.join(format!("session-2026{:04}-100000-{:06}", s, s));
            let slug = snap.join("projects").join("-home-charlie");
            fs::create_dir_all(&slug).unwrap();
            for k in 0..sessions_per_snapshot {
                // Session UUIDs have to look uuid-ish enough for the
                // 8-char short-id slice; pad with zeros.
                let uuid = format!(
                    "{:08x}-snap{:02}-sess{:02}-0000-000000000000",
                    s * 1000 + k,
                    s,
                    k
                );
                let session = slug.join(format!("{uuid}.jsonl"));
                let line = format!(
                    "{{\"role\":\"user\",\"content\":\"hello\",\
                     \"timestamp\":\"2026-04-29T10:00:0{}Z\"}}\n",
                    k % 10
                );
                fs::write(&session, line).unwrap();
            }
        }
        vault
    }

    /// Drive a backfill against an isolated codex dir + fake vault.
    /// Returns the `TempDir` so the caller's scope keeps it alive
    /// (dropping it cleans up the codex + vault on test exit). The
    /// previous version called `tmp.keep()` which leaked the directory
    /// onto disk on every test run — fixed per S2 in PR 272 review.
    fn drive_backfill(
        n_snaps: usize,
        sess_per_snap: usize,
    ) -> (BackfillReport, PathBuf, tempfile::TempDir) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex_dir = tmp.path().join("codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let vault = build_fake_vault(tmp.path(), n_snaps, sess_per_snap);

        let prev = std::env::var("MX_CODEX_PATH").ok();
        // SAFETY: process-wide env mutation, serialized via ENV_LOCK +
        // #[serial] so concurrent codex tests stay correct.
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_dir);
        }
        let options = ArchiveOptions::default();
        let report = run_backfill(&vault, options).expect("backfill failed");

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }

        (report, codex_dir, tmp)
    }

    /// Add a "corrupt" session JSONL to an existing vault snapshot. The
    /// file is unreadable (mode 000) so the archive pipeline's
    /// `read_to_string` returns permission-denied — a realistic shape
    /// for the failure mode S3 cares about.
    ///
    /// Unix-only: the chmod-000 trick relies on POSIX permission bits.
    /// Windows uses a different ACL model where there is no clean
    /// equivalent of "owner cannot read its own file", so we skip the
    /// failure-mode coverage there.
    #[cfg(unix)]
    fn drop_unreadable_session(vault_path: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        // Pick the first snapshot's slug dir.
        let snap = fs::read_dir(vault_path)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("session-"))
            })
            .expect("at least one snapshot");
        let slug = snap.path().join("projects").join("-home-charlie");
        let path = slug.join("deadbeef-corrupt-uuid-0000-000000000000.jsonl");
        fs::write(&path, "this content cannot be read because chmod 000").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// S3 from PR 272 review: a corrupt session JSONL in the vault must
    /// not abort the whole backfill. The error is recorded on
    /// `report.errors`, the other (valid) sessions still archive, and
    /// `run_backfill` returns Ok overall — the non-fatal error model
    /// must hold.
    ///
    /// Unix-only: depends on `drop_unreadable_session` which uses
    /// `chmod 000` to simulate the unreadable case. See helper for the
    /// rationale on Windows.
    #[cfg(unix)]
    #[test]
    #[serial]
    fn backfill_per_session_failure_is_non_fatal() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex_dir = tmp.path().join("codex");
        fs::create_dir_all(&codex_dir).unwrap();

        // 3 valid sessions across 3 snapshots (one each), then drop one
        // corrupt session into the first snapshot for a total of 4
        // sessions found (3 valid + 1 corrupt).
        let vault = build_fake_vault(tmp.path(), 3, 1);
        let corrupt = drop_unreadable_session(&vault);

        let prev = std::env::var("MX_CODEX_PATH").ok();
        // SAFETY: process-wide env mutation, serialized via ENV_LOCK +
        // #[serial] so concurrent codex tests stay correct.
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_dir);
        }

        let report = run_backfill(&vault, ArchiveOptions::default())
            .expect("backfill must return Ok even when a session fails");

        // Restore permissions so TempDir cleanup doesn't choke.
        use std::os::unix::fs::PermissionsExt;
        if corrupt.exists() {
            let mut perms = fs::metadata(&corrupt).unwrap().permissions();
            perms.set_mode(0o644);
            let _ = fs::set_permissions(&corrupt, perms);
        }

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }

        // 4 sessions found (3 valid + 1 corrupt), 3 archived.
        assert_eq!(report.sessions_found, 4);
        assert_eq!(report.sessions_archived, 3);
        assert_eq!(report.sessions_skipped_already_archived, 0);

        // Exactly one per-session error, and it points at the corrupt
        // file.
        assert_eq!(
            report.errors.len(),
            1,
            "expected exactly one error (the corrupt session), got: {:?}",
            report
                .errors
                .iter()
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect::<Vec<_>>()
        );
        assert_eq!(report.errors[0].0, corrupt);
    }

    #[test]
    #[serial]
    fn backfill_walks_snapshots_and_archives_sessions() {
        let (report, codex, _tmp) = drive_backfill(3, 2);
        assert_eq!(report.vault_snapshots_walked, 3);
        assert_eq!(report.sessions_found, 6);
        assert_eq!(report.sessions_archived, 6);
        assert_eq!(report.sessions_skipped_already_archived, 0);
        assert!(report.errors.is_empty());

        // Codex dir should now contain 6 archive directories (one per
        // session) — count the manifest.json files.
        let manifests = fs::read_dir(&codex)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("manifest.json").exists())
            .count();
        assert_eq!(manifests, 6);
    }

    #[test]
    #[serial]
    fn backfill_is_idempotent_on_second_run() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex_dir = tmp.path().join("codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let vault = build_fake_vault(tmp.path(), 2, 2);

        let prev = std::env::var("MX_CODEX_PATH").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_dir);
        }

        let r1 = run_backfill(&vault, ArchiveOptions::default()).unwrap();
        assert_eq!(r1.sessions_archived, 4);
        assert_eq!(r1.sessions_skipped_already_archived, 0);

        let r2 = run_backfill(&vault, ArchiveOptions::default()).unwrap();
        // Second run: nothing new should be archived; everything is a
        // dedup hit.
        assert_eq!(r2.sessions_archived, 0);
        assert_eq!(r2.sessions_skipped_already_archived, 4);

        // The codex directory should still hold exactly the archives
        // from the first pass — no duplicates.
        let manifests = fs::read_dir(&codex_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("manifest.json").exists())
            .count();
        assert_eq!(manifests, 4);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }
    }

    #[test]
    fn backfill_missing_vault_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        let err = run_backfill(&bogus, ArchiveOptions::default()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("does not exist"),
            "expected helpful error, got: {msg}"
        );
    }

    /// W1 from the PR 272 review: if `collect_archived_ids` fails (e.g.,
    /// the codex path points at a regular file rather than a directory,
    /// simulating a permission-denied or corrupted-tree state), the old
    /// code silently swallowed the error via `unwrap_or_default()` and
    /// proceeded with zero dedup — risking double-archives on re-run.
    /// The new behavior surfaces the failure on the report and continues
    /// with an empty seen set so the bulk operation still completes.
    #[test]
    #[serial]
    fn backfill_surfaces_codex_scan_failure_on_report() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();

        // Codex path is a *file*, not a directory. `read_dir` will return
        // NotADirectory, which now propagates to `BackfillReport.errors`.
        let codex_path = tmp.path().join("codex-not-a-dir");
        fs::write(&codex_path, "i am a file, not a directory").unwrap();

        let vault = build_fake_vault(tmp.path(), 1, 1);

        let prev = std::env::var("MX_CODEX_PATH").ok();
        // SAFETY: process-wide env mutation, serialized via ENV_LOCK +
        // #[serial] so concurrent codex tests stay correct.
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_path);
        }

        let report = run_backfill(&vault, ArchiveOptions::default())
            .expect("backfill must continue past a codex scan failure (non-fatal error model)");

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }

        // The codex-scan failure must show up on the report so the
        // operator sees it. There may be additional per-session errors
        // (the per-session archive path may also choke on the
        // not-a-directory codex), but we require the scan failure to be
        // present.
        let scan_err_present = report.errors.iter().any(|(_, e)| {
            let msg = format!("{e:#}");
            msg.contains("scanning codex") || msg.contains("dedup set")
        });
        assert!(
            scan_err_present,
            "expected a codex-scan failure in report.errors, got: {:?}",
            report
                .errors
                .iter()
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[serial]
    fn backfill_empty_vault_warns_and_reports_zeros() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("empty-vault");
        fs::create_dir_all(&vault).unwrap();
        // No session-* children — should warn and return zeros.
        let codex_dir = tmp.path().join("codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let prev = std::env::var("MX_CODEX_PATH").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_dir);
        }

        let report = run_backfill(&vault, ArchiveOptions::default()).unwrap();
        assert_eq!(report.vault_snapshots_walked, 0);
        assert_eq!(report.sessions_found, 0);
        assert_eq!(report.sessions_archived, 0);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }
    }
}
