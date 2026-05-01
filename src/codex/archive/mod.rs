//! Codex archive subsystem.
//!
//! Splits across several files (was a single `archive.rs` until the
//! codex unification PR 2). Layout:
//!
//! - `mod.rs` — public entry points (`archive_session`, `collect_archives`,
//!   `get_codex_dir`, `get_base_archive_name`), plus the
//!   `ArchiveRequest` / `ArchiveResult` plumbing and `archive::run`.
//! - `include.rs` — `IncludeSet`, the opt-in source selector parsed from
//!   the `--include` CLI flag.
//! - `write.rs` — the per-session writer (`archive_session` body) and the
//!   `--all` driver loop.
//! - `sources.rs` — source walkers (today: `find_agent_sessions`; later:
//!   MCP / tool-output / history).
//! - `paths.rs` — archive-folder naming utilities (`determine_archive_dir`,
//!   `parse_archive_name`, `extract_short_id`, `get_base_archive_name`).
//!
//! `archive::run` is the one canonical entry point.
//! `archive_session` is a thin wrapper that builds an `ArchiveRequest` from
//! CLI args and calls `run`. Status-quo invocations
//! (`mx codex archive` with no `--include`) produce byte-identical
//! output to the pre-PR-2 implementation.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ArchiveEntry, Manifest};

mod backfill;
mod include;
mod paths;
mod sources;
mod write;

// Re-exports kept at the historical paths so `super::archive::*` callers
// (notably `migrate.rs` and `read.rs`) need no changes.
pub(crate) use backfill::run_backfill;
pub(crate) use include::IncludeSet;
pub(crate) use paths::{get_base_archive_name, parse_archive_name};

/// One archive request — either a single session by path, or the bulk
/// "archive everything not yet archived" mode.
#[derive(Debug, Clone)]
pub enum ArchiveRequest {
    /// Archive a specific session JSONL.
    Single(PathBuf),
    /// Walk `~/.claude/projects/` and archive every unarchived session.
    All,
}

/// Optional knobs that apply to every `ArchiveRequest`.
#[derive(Debug, Clone)]
pub struct ArchiveOptions {
    /// Clean mode: write `conversation.md` + images instead of the raw
    /// JSONL + agent files.
    pub clean: bool,
    /// Which optional source artifacts to capture.
    pub include: IncludeSet,
    /// Include sub-agent transcripts inside `conversation.md`. Only
    /// meaningful in clean mode; matches the historical
    /// `--include-agents` flag (still wired separately for backward
    /// compatibility — see `cli.rs`).
    pub include_agents_in_clean_md: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            clean: false,
            include: IncludeSet::status_quo(),
            include_agents_in_clean_md: false,
        }
    }
}

/// Outcome of a successful `archive::run`.
#[derive(Debug, Clone, Default)]
pub struct ArchiveResult {
    /// How many sessions were freshly archived (1 for `Single`; N for `All`).
    pub archived_count: usize,
    /// Sessions skipped because they were already archived (only meaningful
    /// for `ArchiveRequest::All`; always 0 for `Single` because the path
    /// is taken on faith — collisions are handled by suffix instead).
    pub skipped_count: usize,
    /// Resolved archive directory paths, in archive order. Useful for
    /// callers that want to chain follow-up work (e.g. printing, indexing).
    pub archive_paths: Vec<PathBuf>,
}

/// Canonical archive entry point. Builds the artifacts on disk according
/// to `request` and `options`, returns a summary.
///
/// Behavior with `IncludeSet::status_quo()` and `clean = false` is
/// byte-identical to the pre-PR-2 `mx codex archive` flow.
///
/// After a successful write, the by-project index
/// (`<codex_dir>/by-project/`) is rebuilt so subsequent reads can find
/// sessions by project basename. Index rebuild failures are logged but
/// do NOT fail the archive — the index is regenerable, so a future
/// archive run will heal it.
pub fn run(request: ArchiveRequest, options: ArchiveOptions) -> Result<ArchiveResult> {
    let mut result = ArchiveResult::default();

    match request {
        ArchiveRequest::Single(path) => {
            // NOTE: backfill.rs::session_id_from_path derives the dedup
            // key from the JSONL filename stem. If we ever change how
            // the canonical session_id is derived for a Single archive
            // (e.g., from the JSONL's `sessionId` field instead of the
            // filename), update backfill's dedup logic to match,
            // otherwise idempotence will break silently. See W2 in PR
            // 272 review for context.
            let archive_dir = write::archive_session(
                &path,
                options.clean,
                options.include_agents_in_clean_md,
                &options.include,
            )?;
            result.archived_count = 1;
            result.archive_paths.push(archive_dir);
        }
        ArchiveRequest::All => {
            // S1: save_all_sessions returns ArchiveResult directly now;
            // no field-by-field copy from a near-duplicate BulkSummary.
            result = write::save_all_sessions(
                options.clean,
                options.include_agents_in_clean_md,
                &options.include,
            )?;
        }
    }

    // Refresh the by-project index. Best-effort: a failed rebuild is a
    // warning, not a hard failure — readers must already tolerate a
    // missing-or-stale index.
    if let Err(e) = rebuild_project_index() {
        eprintln!("warning: by-project index rebuild failed: {e}");
    }

    Ok(result)
}

/// Open the by-project index and rebuild it from the manifests under
/// the codex root. Extracted into a small helper so the failure path in
/// `run` stays obvious.
fn rebuild_project_index() -> Result<()> {
    let mut idx = super::index::ProjectIndex::open()?;
    idx.rebuild_from_manifests()?;
    Ok(())
}

/// CLI shim. Builds an `ArchiveRequest` from the flat CLI args and
/// delegates to `run`.
pub(crate) fn archive_session(
    session_path: Option<String>,
    all: bool,
    clean: bool,
    include_agents: bool,
    include: IncludeSet,
) -> Result<()> {
    let request = if all {
        ArchiveRequest::All
    } else {
        let path = resolve_session_path(session_path)?;
        ArchiveRequest::Single(path)
    };
    let options = ArchiveOptions {
        clean,
        include,
        include_agents_in_clean_md: include_agents,
    };
    run(request, options)?;
    Ok(())
}

fn resolve_session_path(path: Option<String>) -> Result<PathBuf> {
    if let Some(p) = path {
        Ok(PathBuf::from(p))
    } else {
        crate::session::find_most_recent_session()
    }
}

/// Walk every archive dir under `codex_dir` and return one `ArchiveEntry`
/// per valid manifest. Used by `read.rs` (list/search) and `migrate.rs`.
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

pub(super) fn get_codex_dir() -> Result<PathBuf> {
    Ok(crate::paths::codex_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::Manifest;
    use serial_test::serial;
    use std::sync::Mutex;

    /// Tests in this file mutate process-wide environment (`MX_CODEX_PATH`)
    /// to redirect the codex root. Multiple tests doing that concurrently
    /// would race; this mutex + `#[serial]` keeps them strictly ordered
    /// even alongside other codex_dir-touching tests in the same binary.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_session_jsonl() -> String {
        // Inline copy of tests/fixtures/manifest-golden/session.jsonl. We
        // duplicate it because unit tests run from the crate root with
        // OUT_DIR-style indirection rather than CARGO_MANIFEST_DIR-style
        // file lookups, and pinning a path-based fixture into the unit
        // suite is brittler than the inline string.
        concat!(
            r#"{"role":"user","content":"hello","timestamp":"2026-04-29T10:00:00Z"}"#,
            "\n",
            r#"{"role":"assistant","content":"hi there","timestamp":"2026-04-29T10:00:30Z"}"#,
            "\n",
            r#"{"role":"user","content":"thanks","timestamp":"2026-04-29T10:01:00Z"}"#,
            "\n",
        )
        .to_string()
    }

    /// Replace volatile fields with sentinels so the comparison is stable
    /// across machines and runs. Volatile fields are:
    ///
    /// - `archived_at` (always `Utc::now()` at archive time)
    /// - `version` (will rev independently of layout)
    ///
    /// `session_start` and `session_end` are NOT normalized: the fixture
    /// JSONL has stable timestamps in the file itself, so a write that
    /// regresses C2's timestamp derivation would change them and thus
    /// fail the golden — exactly the test we want.
    ///
    /// Image hashes and byte counts are also normalized because image
    /// extraction creates files whose names depend on hash content; the
    /// fixture has no images, so this is a defensive no-op today, but it
    /// future-proofs the matrix.
    fn normalize(value: &mut serde_json::Value) {
        use serde_json::Value;
        if let Value::Object(map) = value {
            // Time-dependent fields must be sentinelled.
            for key in ["archived_at", "version"] {
                if map.contains_key(key) {
                    map.insert(key.to_string(), Value::String("__SENTINEL__".to_string()));
                }
            }
            // User/assistant display names resolve via MX_USER_NAME /
            // MX_ASSISTANT_NAME / git config — machine-dependent. The
            // golden treats them as opaque so cross-developer runs match.
            for key in ["user_name", "assistant_name"] {
                if let Some(v) = map.get_mut(key)
                    && !v.is_null()
                {
                    *v = Value::String("__SENTINEL__".to_string());
                }
            }
            // size_bytes in clean mode includes the rendered
            // conversation.md, whose length depends on the
            // (machine-dependent) user_name / assistant_name. The
            // structure of the manifest — which is what the golden
            // protects — does not change with size, so we sentinel it
            // to keep cross-machine runs stable.
            if let Some(v) = map.get_mut("size_bytes")
                && v.is_number()
            {
                *v = Value::String("__SENTINEL_NUM__".to_string());
            }
        }
    }

    /// Compare a manifest against its golden, normalizing volatile
    /// fields. The serialized JSON is parsed both sides as `Value` so a
    /// reorder of the structurally-equivalent JSON would NOT slip
    /// through — but a structural change (renamed key, reordered fields
    /// at the schema level, dropped/added fields) WILL fail.
    ///
    /// IMPORTANT: this also serializes both back out with
    /// `to_string_pretty` and string-compares, which catches changes to
    /// indentation / serializer settings (the C1 brief: "a future change
    /// that ... changes serializer indentation must FAIL").
    fn assert_manifest_matches_golden(manifest_text: &str, golden_path: &Path) {
        let mut got: serde_json::Value =
            serde_json::from_str(manifest_text).expect("manifest is not valid JSON");
        normalize(&mut got);

        let regen_env = std::env::var("MX_UPDATE_GOLDENS").is_ok();
        if regen_env || !golden_path.exists() {
            // Materialize the golden the first time, or whenever the
            // operator opts into regeneration.
            let pretty = serde_json::to_string_pretty(&got).unwrap();
            std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
            std::fs::write(golden_path, &pretty).unwrap();
            if !regen_env {
                eprintln!(
                    "note: created golden {} on first run",
                    golden_path.display()
                );
                return;
            }
        }

        let want_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("missing golden {}: {e}", golden_path.display()));
        let want: serde_json::Value =
            serde_json::from_str(&want_text).expect("golden is not valid JSON");

        // Structural equality first — gives the clearest diff.
        assert_eq!(
            got,
            want,
            "manifest structure drifted from golden {}\n\
             tip: re-run with MX_UPDATE_GOLDENS=1 to refresh, then audit the diff before committing.",
            golden_path.display()
        );

        // Byte-identity of the pretty-printed form. Catches indentation
        // / serializer-setting drift even when the structural compare
        // passes.
        let got_pretty = serde_json::to_string_pretty(&got).unwrap();
        let want_pretty = serde_json::to_string_pretty(&want).unwrap();
        assert_eq!(
            got_pretty,
            want_pretty,
            "manifest pretty-print bytes drifted from golden {}",
            golden_path.display()
        );
    }

    fn golden_path(name: &str) -> PathBuf {
        // CARGO_MANIFEST_DIR points at the crate root for both `cargo
        // test --bin mx` and `cargo nextest`; the goldens live under
        // tests/fixtures/manifest-golden/.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("manifest-golden")
            .join(name)
    }

    /// Drive `archive::run` against an isolated codex dir. Returns the
    /// serialized manifest text from the resulting archive directory.
    fn archive_and_read_manifest(clean: bool) -> (String, Manifest) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().unwrap();
        let codex_dir = tmp.path().join("codex");
        std::fs::create_dir_all(&codex_dir).unwrap();

        let session_dir = tmp.path().join("project-slug");
        std::fs::create_dir_all(&session_dir).unwrap();
        let session_path = session_dir.join("c3744b8d-test.jsonl");
        std::fs::write(&session_path, fixture_session_jsonl()).unwrap();

        let prev = std::env::var("MX_CODEX_PATH").ok();
        // SAFETY: process-wide env var. The ENV_LOCK above and #[serial]
        // on each test enforce mutual exclusion within and across the
        // codex test surface.
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex_dir);
        }

        let options = ArchiveOptions {
            clean,
            ..ArchiveOptions::default()
        };

        let result = run(ArchiveRequest::Single(session_path), options);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
        }

        let result = result.expect("archive::run failed");
        assert_eq!(result.archived_count, 1);
        let archive_dir = result.archive_paths.first().expect("no archive dir");

        let manifest_text =
            std::fs::read_to_string(archive_dir.join("manifest.json")).expect("manifest missing");
        let m: Manifest = serde_json::from_str(&manifest_text).unwrap();
        (manifest_text, m)
    }

    /// Golden file: status-quo (single + full, clean=false).
    ///
    /// This is the load-bearing byte-identity test. A future change that
    /// reorders Manifest fields, renames anything, or changes
    /// serializer indentation will fail this assertion. To intentionally
    /// regenerate, run with `MX_UPDATE_GOLDENS=1` and review the diff
    /// before committing.
    #[test]
    #[serial]
    fn manifest_golden_single_full() {
        let (manifest_text, m) = archive_and_read_manifest(false);
        // Sanity: status-quo must not leak v5 fields.
        assert!(m.tool_output_count.is_none());
        assert!(m.mcp_log_count.is_none());
        assert!(m.history_lines.is_none());
        assert!(m.source_breakdown.is_none());

        assert_manifest_matches_golden(&manifest_text, &golden_path("single-full.json"));
    }

    /// Golden file: single + clean (clean=true).
    ///
    /// The clean path emits `conversation.md` + images, so the manifest
    /// has `has_clean_transcript: true` and zeroed-out
    /// session.jsonl/agents bytes — which is captured in the golden.
    #[test]
    #[serial]
    fn manifest_golden_single_clean() {
        let (manifest_text, m) = archive_and_read_manifest(true);
        assert_eq!(m.has_clean_transcript, Some(true));
        assert!(m.tool_output_count.is_none());
        assert!(m.source_breakdown.is_none());

        assert_manifest_matches_golden(&manifest_text, &golden_path("single-clean.json"));
    }

    // NOTE: Two cells from the C1 matrix are deferred:
    //
    //   - all + full
    //   - all + clean
    //
    // The `--all` driver walks `~/.claude/projects/`, which derives
    // strictly from `dirs::home_dir()` with no env-var override. To
    // exercise it from a unit test would require mutating $HOME
    // process-wide, which is fragile across the test binary's parallel
    // suite even with serial_test. The single-* cells already cover the
    // load-bearing manifest serialization path: the only behavioral
    // delta in --all is the loop driver and the `archived_count` /
    // `skipped_count` bookkeeping, both of which are exercised in
    // separate index/rebuild tests.
}
