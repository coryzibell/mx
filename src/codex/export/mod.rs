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
use std::path::PathBuf;

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
    /// Output file path. `None` means stdout for markdown / JSON; for
    /// `Format::Both` an output path is required (we route JSON to the
    /// file and markdown to stderr).
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

/// Canonical export entry point.
pub fn run(request: ExportRequest) -> Result<ExportResult> {
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
            // For `both`, JSON goes to the output (file or stdout) and
            // markdown is sent to stderr commentary. The brief calls
            // out that pure stdout for json+markdown is confusing.
            let json_body = join_json(&json_chunks);
            let md_body = join_markdown(&markdown_chunks);
            let p = emit(&request.output, &json_body)?;
            eprintln!("{}", md_body);
            p
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
}
