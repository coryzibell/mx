//! `mx codex export` — read sessions out of the codex and emit them.
//!
//! Mirrors the shape of `archive::run`: build an `ExportRequest`, call
//! `export::run`, get an `ExportResult`. The CLI handler does the
//! parameter parsing.
//!
//! Architectural invariants (enforced here):
//!
//! - Content is read from `<codex_dir>/<archive_dir>/` ONLY. The
//!   detection layer scans `~/.claude/` for the warning, but no
//!   rendering ever ingests live Claude data — that's PR 2's domain
//!   (archive).
//! - `--archive-first` short-circuits the warning by running
//!   `archive::run(ArchiveRequest::All, _)` first, then re-detecting,
//!   then exporting.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub mod detect;
pub mod filter;
pub mod format;
pub mod include;
pub mod read;

pub use filter::{DateRange, Selector, SessionRef};
pub use format::Format;
pub use include::ExportIncludeSet;

/// What the caller wants exported.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub selector: Selector,
    pub format: Format,
    pub include: ExportIncludeSet,
    /// If set, run `mx codex archive --all` before exporting and skip
    /// the unarchived-data warning.
    pub archive_first: bool,
    /// Output file path. `None` means stdout for markdown / JSON.
    ///
    /// `Format::Both` requires this to be `Some`: JSON is written to the
    /// supplied path and markdown is written to a sibling sidecar file
    /// (see [`both_sidecar_paths`] for the naming rule). Calling `run`
    /// with `Format::Both` and `output: None` returns an error rather
    /// than silently mixing JSON on stdout with markdown on stderr.
    pub output: Option<PathBuf>,
}

/// Outcome of a successful `export::run`.
#[derive(Debug, Clone, Default)]
pub struct ExportResult {
    /// How many archive directories were rendered.
    pub session_count: usize,
    /// The detection report (post-archive-first re-run if applicable).
    pub detection: detect::DetectionReport,
    /// Where output was written (file path) or `None` for stdout.
    pub output_path: Option<PathBuf>,
}

/// Compute the sidecar file pair for `Format::Both`.
///
/// Rule:
/// - If `path` ends in `.json`: JSON goes to `path`, markdown goes to
///   `path.with_extension("md")`.
/// - If `path` ends in `.md`: markdown goes to `path`, JSON goes to
///   `path.with_extension("json")`.
/// - Otherwise: JSON goes to `<path>.json`, markdown goes to `<path>.md`
///   (the original `path` is preserved as a stem).
///
/// Returns `(json_path, markdown_path)`.
pub fn both_sidecar_paths(path: &Path) -> (PathBuf, PathBuf) {
    let ext = path.extension().and_then(|s| s.to_str());
    match ext {
        Some("json") => (path.to_path_buf(), path.with_extension("md")),
        Some("md") => (path.with_extension("json"), path.to_path_buf()),
        _ => {
            // Append the typed extension to whatever stem the operator
            // gave us (e.g. `out` → `out.json` + `out.md`, or
            // `out.txt` → `out.txt.json` + `out.txt.md`). We deliberately
            // do NOT strip arbitrary extensions here — that would be
            // surprising for paths like `archive.tar` where the operator
            // expects the name preserved verbatim.
            let mut json_path = path.as_os_str().to_owned();
            json_path.push(".json");
            let mut md_path = path.as_os_str().to_owned();
            md_path.push(".md");
            (PathBuf::from(json_path), PathBuf::from(md_path))
        }
    }
}

/// Canonical export entry point.
pub fn run(request: ExportRequest) -> Result<ExportResult> {
    // -------- Step 0: validate --------
    if matches!(request.format, Format::Both) && request.output.is_none() {
        anyhow::bail!(
            "--format both requires --output (writes <out>.json and <out>.md sidecar files; \
             pure stdout would mix JSON on stdout with markdown on stderr)"
        );
    }

    // -------- Step 1: optional pre-archive --------
    if request.archive_first {
        let archive_request = crate::codex::archive::ArchiveRequest::All;
        let archive_options = crate::codex::archive::ArchiveOptions::default();
        crate::codex::archive::run(archive_request, archive_options)
            .context("--archive-first: archive::run(All) failed")?;
    }

    // -------- Step 2: detect unarchived data --------
    let detection = detect::detect_unarchived().unwrap_or_default();
    if !request.archive_first
        && let Some(warn) = detection.warning_text()
    {
        eprintln!("{}", warn);
    }

    // -------- Step 3: resolve selector --------
    let codex_dir = crate::paths::codex_dir();
    let all_archives = filter::collect_codex_archives(&codex_dir)?;

    let archives = match &request.selector {
        Selector::Latest => vec![filter::resolve_latest(all_archives)?],
        Selector::Session(sref) => vec![filter::resolve_session(all_archives, sref)?],
        Selector::Project(query) => filter::resolve_project(all_archives, query)?,
        Selector::Date(range) => {
            let matched = filter::resolve_date(all_archives, range);
            if matched.is_empty() {
                anyhow::bail!(
                    "no archived sessions fall in date range [{} .. {})",
                    range.start.to_rfc3339(),
                    range.end.to_rfc3339()
                );
            }
            matched
        }
    };

    // -------- Step 4: render each archive --------
    let mut markdown_chunks: Vec<String> = Vec::new();
    let mut json_chunks: Vec<String> = Vec::new();
    for resolved in &archives {
        let loaded = read::read_archive(&resolved.archive_dir)?;
        match request.format {
            Format::Markdown => {
                markdown_chunks.push(format::markdown::render(
                    &loaded,
                    &resolved.manifest,
                    &request.include,
                )?);
            }
            Format::Json => {
                json_chunks.push(format::json::render(
                    &loaded,
                    &resolved.manifest,
                    &request.include,
                )?);
            }
            Format::Both => {
                markdown_chunks.push(format::markdown::render(
                    &loaded,
                    &resolved.manifest,
                    &request.include,
                )?);
                json_chunks.push(format::json::render(
                    &loaded,
                    &resolved.manifest,
                    &request.include,
                )?);
            }
        }
    }

    // -------- Step 5: emit --------
    let output_path = match request.format {
        Format::Markdown => {
            let body = join_markdown(&markdown_chunks);
            emit(&request.output, &body)?
        }
        Format::Json => {
            let body = join_json(&json_chunks);
            emit(&request.output, &body)?
        }
        Format::Both => {
            // Step 0 has guaranteed `output.is_some()`. Split into two
            // sidecar files so neither stream stomps on the other.
            let path = request
                .output
                .as_ref()
                .expect("Format::Both with no output should have been rejected at step 0");
            let (json_path, md_path) = both_sidecar_paths(path);
            let json_body = join_json(&json_chunks);
            let md_body = join_markdown(&markdown_chunks);
            std::fs::write(&json_path, &json_body)
                .with_context(|| format!("write export JSON to {}", json_path.display()))?;
            std::fs::write(&md_path, &md_body)
                .with_context(|| format!("write export markdown to {}", md_path.display()))?;
            // Report the JSON path as the "primary" output_path for
            // backwards-compatibility with callers that already inspect
            // it; the markdown sidecar is implied by the rule.
            Some(json_path)
        }
    };

    Ok(ExportResult {
        session_count: archives.len(),
        detection,
        output_path,
    })
}

fn join_markdown(chunks: &[String]) -> String {
    if chunks.len() == 1 {
        return chunks[0].clone();
    }
    chunks.join("\n\n---\n\n")
}

fn join_json(chunks: &[String]) -> String {
    // Multiple sessions → wrap in an array. Re-parse so the result is
    // always valid JSON (concatenating pretty-printed objects with `,`
    // would not be).
    if chunks.len() == 1 {
        return chunks[0].clone();
    }
    let parsed: Vec<serde_json::Value> = chunks
        .iter()
        .filter_map(|c| serde_json::from_str(c).ok())
        .collect();
    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| "[]".to_string())
}

fn emit(output: &Option<PathBuf>, body: &str) -> Result<Option<PathBuf>> {
    match output {
        Some(path) => {
            std::fs::write(path, body)
                .with_context(|| format!("write export output to {}", path.display()))?;
            Ok(Some(path.clone()))
        }
        None => {
            std::io::stdout().write_all(body.as_bytes())?;
            // Trailing newline so terminal prompts don't run together
            // with the last line of output.
            if !body.ends_with('\n') {
                std::io::stdout().write_all(b"\n")?;
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::MANIFEST_WRITE_VERSION;
    use chrono::Utc;
    use serial_test::serial;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_archive(codex_dir: &std::path::Path, dir_name: &str, session_id: &str) {
        let archive_dir = codex_dir.join(dir_name);
        std::fs::create_dir_all(&archive_dir).unwrap();
        let manifest = crate::codex::Manifest {
            version: MANIFEST_WRITE_VERSION,
            session_id: session_id.to_string(),
            archived_at: Utc::now(),
            session_start: Utc::now(),
            session_end: Utc::now(),
            project_path: Some("/home/charlie/work/mx".to_string()),
            message_count: 1,
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
        std::fs::write(
            archive_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            archive_dir.join("session.jsonl"),
            r#"{"type":"user","message":{"content":"hi"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}
"#,
        )
        .unwrap();
    }

    /// Run an export with `MX_CODEX_PATH` and `MX_CLAUDE_PROJECTS_DIR`
    /// pointed at temp dirs.
    #[test]
    #[serial]
    fn export_latest_writes_markdown_to_file() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let projects = tmp.path().join("claude-projects-sentinel");
        let out_path = tmp.path().join("out.md");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        // SAFETY: env mutation guarded by ENV_LOCK + #[serial].
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &projects);
        }

        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Markdown,
            include: ExportIncludeSet::default_clean(),
            archive_first: false,
            output: Some(out_path.clone()),
        };
        let result = run(req);

        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let result = result.expect("export::run failed");
        assert_eq!(result.session_count, 1);
        assert_eq!(result.output_path.as_deref(), Some(out_path.as_path()));
        let body = std::fs::read_to_string(&out_path).unwrap();
        assert!(body.contains("Session aaaaaaaa-1111"));
        assert!(body.contains("hello"));
    }

    #[test]
    #[serial]
    fn export_does_not_read_claude_projects_for_content() {
        // Architectural invariant: export reads content from the codex
        // exclusively. We point MX_CLAUDE_PROJECTS_DIR at a sentinel
        // path containing a session JSONL that, if read, would obviously
        // collide with the codex archive (different UUID, different
        // body). The export must not surface anything from the sentinel.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let sentinel = tmp.path().join("claude-projects-sentinel");
        std::fs::create_dir_all(&codex).unwrap();
        let proj_subdir = sentinel.join("-home-charlie-mx");
        std::fs::create_dir_all(&proj_subdir).unwrap();
        // Plant a "live but not archived" session in the sentinel.
        std::fs::write(
            proj_subdir.join("ffffffff-9999.jsonl"),
            r#"{"type":"user","message":{"content":"LIVE_DATA_SHOULD_NOT_LEAK"}}
"#,
        )
        .unwrap();
        // And one archived session in the codex.
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &sentinel);
        }
        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Markdown,
            include: ExportIncludeSet::default_clean(),
            archive_first: false,
            output: Some(tmp.path().join("out.md")),
        };
        let result = run(req);
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let result = result.expect("export::run failed");
        let body = std::fs::read_to_string(result.output_path.as_ref().unwrap()).unwrap();
        // The codex archive's content should be present.
        assert!(body.contains("Session aaaaaaaa-1111"));
        // The sentinel's content must NEVER be in the output.
        assert!(
            !body.contains("LIVE_DATA_SHOULD_NOT_LEAK"),
            "export read live ~/.claude/projects/ content — invariant violated"
        );
        // The detection report should have flagged the live session as
        // unarchived (a side-effect signal that the detection scan ran).
        assert!(result.detection.unarchived_session_count >= 1);
    }

    /// S1: same architectural invariant as the markdown test, but for
    /// the JSON emitter. The JSON walker recurses into every string
    /// field — if it ever read live `~/.claude/projects/` content, the
    /// sentinel would surface in some string somewhere in the document.
    /// We parse the output and walk every string value, asserting the
    /// sentinel never appears.
    #[test]
    #[serial]
    fn export_does_not_read_claude_projects_for_content_json() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let sentinel = tmp.path().join("claude-projects-sentinel");
        std::fs::create_dir_all(&codex).unwrap();
        let proj_subdir = sentinel.join("-home-charlie-mx");
        std::fs::create_dir_all(&proj_subdir).unwrap();
        // Same fixture path / sentinel value as the markdown test, so a
        // regression in either emitter looks the same in the failure.
        std::fs::write(
            proj_subdir.join("ffffffff-9999.jsonl"),
            r#"{"type":"user","message":{"content":"LIVE_DATA_SHOULD_NOT_LEAK"}}
"#,
        )
        .unwrap();
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &sentinel);
        }
        let out_path = tmp.path().join("out.json");
        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Json,
            include: ExportIncludeSet::default_clean(),
            archive_first: false,
            output: Some(out_path.clone()),
        };
        let result = run(req);
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let result = result.expect("export::run failed");
        let body = std::fs::read_to_string(result.output_path.as_ref().unwrap()).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("JSON output must parse");

        // Walk every string in the JSON tree and assert the sentinel
        // never appears. This catches the case where a future field
        // surfaces live content in some unanticipated path (e.g. an
        // image caption or a subagent excerpt).
        fn assert_no_sentinel(v: &serde_json::Value, path: &str) {
            match v {
                serde_json::Value::String(s) => {
                    assert!(
                        !s.contains("LIVE_DATA_SHOULD_NOT_LEAK"),
                        "sentinel leaked into JSON at {path}: {s}"
                    );
                }
                serde_json::Value::Array(arr) => {
                    for (i, item) in arr.iter().enumerate() {
                        assert_no_sentinel(item, &format!("{path}[{i}]"));
                    }
                }
                serde_json::Value::Object(map) => {
                    for (k, val) in map {
                        assert_no_sentinel(val, &format!("{path}.{k}"));
                    }
                }
                _ => {}
            }
        }
        assert_no_sentinel(&parsed, "$");
        // Detection still flagged the live session as unarchived (same
        // side-effect signal as the markdown invariant test).
        assert!(result.detection.unarchived_session_count >= 1);
    }

    #[test]
    #[serial]
    fn export_archive_first_skips_warning() {
        // With `--archive-first`, the warning is NOT printed (because
        // detection is re-run after archiving and should be zero). We
        // can't easily intercept stderr, but we can verify the
        // detection report on the result is the post-archive state.
        // Here we don't actually have any live ~/.claude data, so this
        // is mostly a smoke test for the archive-first path not
        // crashing.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let projects = tmp.path().join("projects");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &projects);
        }
        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Markdown,
            include: ExportIncludeSet::default_clean(),
            archive_first: true,
            output: Some(tmp.path().join("out.md")),
        };
        let result = run(req);
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let result = result.expect("--archive-first export failed");
        // Post-archive-first detection: there's no live ~/.claude/projects/
        // session here, so unarchived count must be zero.
        assert_eq!(result.detection.unarchived_session_count, 0);
    }

    // ---- W1: Format::Both sidecar split ----

    #[test]
    #[serial]
    fn export_format_both_without_output_errors() {
        // `Format::Both` without `--output` must be rejected loudly. The
        // previous behavior routed JSON to stdout and markdown to stderr,
        // which mangled `mx codex export --format both > out.json`.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let projects = tmp.path().join("projects");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &projects);
        }
        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Both,
            include: ExportIncludeSet::default_clean(),
            archive_first: false,
            output: None,
        };
        let result = run(req);
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let err = result.expect_err("Format::Both without --output should error");
        let msg = format!("{err}");
        assert!(
            msg.contains("--format both requires --output"),
            "error message should call out the requirement; got: {msg}"
        );
    }

    #[test]
    #[serial]
    fn export_format_both_writes_sidecar_files() {
        // With `-o foo.json` we expect `foo.json` (JSON) and `foo.md`
        // (markdown) written side by side.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let projects = tmp.path().join("projects");
        let out_json = tmp.path().join("foo.json");
        let out_md = tmp.path().join("foo.md");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        write_archive(&codex, "2026-04-29-100000-aaaaaaaa", "aaaaaaaa-1111");

        let prev_codex = std::env::var("MX_CODEX_PATH").ok();
        let prev_proj = std::env::var("MX_CLAUDE_PROJECTS_DIR").ok();
        unsafe {
            std::env::set_var("MX_CODEX_PATH", &codex);
            std::env::set_var("MX_CLAUDE_PROJECTS_DIR", &projects);
        }
        let req = ExportRequest {
            selector: Selector::Latest,
            format: Format::Both,
            include: ExportIncludeSet::default_clean(),
            archive_first: false,
            output: Some(out_json.clone()),
        };
        let result = run(req);
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("MX_CODEX_PATH", v),
                None => std::env::remove_var("MX_CODEX_PATH"),
            }
            match prev_proj {
                Some(v) => std::env::set_var("MX_CLAUDE_PROJECTS_DIR", v),
                None => std::env::remove_var("MX_CLAUDE_PROJECTS_DIR"),
            }
        }
        let result = result.expect("Format::Both export should succeed with --output");
        assert_eq!(result.output_path.as_deref(), Some(out_json.as_path()));
        assert!(out_json.exists(), "JSON sidecar at .json path must exist");
        assert!(out_md.exists(), "markdown sidecar at .md path must exist");
        let json_body = std::fs::read_to_string(&out_json).unwrap();
        // Must parse as JSON (not be markdown by accident).
        let _: serde_json::Value =
            serde_json::from_str(&json_body).expect("json sidecar must be parseable JSON");
        let md_body = std::fs::read_to_string(&out_md).unwrap();
        assert!(
            md_body.contains("Session aaaaaaaa-1111"),
            "markdown sidecar must contain the rendered conversation"
        );
    }

    #[test]
    fn both_sidecar_paths_extension_rules() {
        let (j, m) = both_sidecar_paths(Path::new("/tmp/foo.json"));
        assert_eq!(j, PathBuf::from("/tmp/foo.json"));
        assert_eq!(m, PathBuf::from("/tmp/foo.md"));

        let (j, m) = both_sidecar_paths(Path::new("/tmp/foo.md"));
        assert_eq!(j, PathBuf::from("/tmp/foo.json"));
        assert_eq!(m, PathBuf::from("/tmp/foo.md"));

        // No extension: append both.
        let (j, m) = both_sidecar_paths(Path::new("/tmp/foo"));
        assert_eq!(j, PathBuf::from("/tmp/foo.json"));
        assert_eq!(m, PathBuf::from("/tmp/foo.md"));

        // Unrelated extension: preserve verbatim and append.
        let (j, m) = both_sidecar_paths(Path::new("/tmp/archive.tar"));
        assert_eq!(j, PathBuf::from("/tmp/archive.tar.json"));
        assert_eq!(m, PathBuf::from("/tmp/archive.tar.md"));
    }
}
