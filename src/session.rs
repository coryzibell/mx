//! `mx session export` — DEPRECATED alias for `mx codex export`.
//!
//! This subcommand used to walk `~/.claude/projects/`, parse the most
//! recent session JSONL, and emit markdown to stdout / a file. As of
//! the codex unification work (#254), all that machinery moved into
//! `mx codex export`, which reads exclusively from the codex.
//!
//! `mx session export` is preserved as a compatibility shim that
//! routes the old CLI args into an `ExportRequest` and delegates to
//! `codex::export::run`. A deprecation warning fires on stderr — but
//! it's a notice, not a block: the old invocation continues to work.
//!
//! `find_most_recent_session` is kept here because `codex::archive`
//! still depends on it for resolving "save the live session right
//! now" without round-tripping through the codex.

use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

/// Stderr-printed deprecation notice for `mx session export`. Kept as a
/// constant so tests can assert the exact phrasing without sniffing
/// stderr from a child process.
pub const DEPRECATION_NOTICE: &str = "\
note: `mx session export` is deprecated; use `mx codex export` instead.
      The new command reads from the codex (run `mx codex archive` first
      if needed; this alias does that for you), supports filtering by
      --session, --project, --date, multiple output formats, and inlines
      sub-agent transcripts by default. Run `mx codex export --help` for
      the full surface.
note: the new command selects \"most recent\" by session start time, not
      by JSONL file mtime. For single-session use cases this matches the
      legacy behavior.";

/// Build the `ExportRequest` that this alias dispatches to
/// `codex::export::run`. Pure — no I/O, no env reads — so the routing
/// logic is unit-testable.
///
/// Translation rules:
///
/// - No positional `path` → `Selector::Latest`. The codex picks the
///   most recent archive by `session_start`.
/// - Positional `path` ending in `.jsonl` → derive the session UUID
///   from the file stem and route to `Selector::Session(SessionRef)`.
///   This matches how the legacy command picked sessions: by the JSONL
///   filename under `~/.claude/projects/`.
/// - `output` flows through verbatim.
/// - `archive_first: true` is forced. The legacy command read live
///   data; the new command reads codex. Without `--archive-first`, an
///   operator who has never run `mx codex archive` would get a "no
///   codex data" error here. Forcing archive-first preserves the
///   user-visible "I just ran the command and it worked" semantics.
/// - `format: Markdown` — the only output the legacy command emitted.
/// - `include: ExportIncludeSet::default_clean()` — clean human
///   conversation. Note this DOES inline sub-agents; the legacy
///   command silently dropped them. Mentioned in DEPRECATION_NOTICE.
pub(crate) fn build_alias_request(
    path: Option<String>,
    output: Option<String>,
) -> Result<crate::codex::ExportRequest> {
    use crate::codex::export::SessionRef;
    use crate::codex::{ExportIncludeSet, ExportRequest, Format, Selector};

    let selector = match path {
        None => Selector::Latest,
        Some(p) => {
            let pb = PathBuf::from(&p);
            // Derive the session UUID from the JSONL filename. The
            // legacy command identified sessions by the file under
            // `~/.claude/projects/<slug>/<uuid>.jsonl`; the codex keys
            // on the same UUID via `manifest.session_id`.
            let stem = pb.file_stem().and_then(|s| s.to_str()).with_context(|| {
                format!(
                    "session export path '{}' has no filename stem (expected <uuid>.jsonl)",
                    p
                )
            })?;

            // Sanity-check the path before routing into the codex,
            // which would otherwise translate "README.md" into a
            // Selector::Session("README") and fail downstream with a
            // codex-flavored "session not found" — confusing for the
            // operator. Accept either a `.jsonl` extension OR a
            // UUID-shaped stem (full 36-char dashed UUID, or the
            // 8-char short form the codex uses internally).
            let ext = pb.extension().and_then(|s| s.to_str());
            let is_jsonl = matches!(ext, Some("jsonl"));
            if !is_jsonl && !is_uuid_shaped(stem) {
                anyhow::bail!(
                    "session export expects a path to a session JSONL file; got: {}\n\
                     hint: try `mx codex export --session <uuid>` instead",
                    p
                );
            }
            Selector::Session(SessionRef(stem.to_string()))
        }
    };

    Ok(ExportRequest {
        selector,
        format: Format::Markdown,
        include: ExportIncludeSet::default_clean(),
        archive_first: true,
        output: output.map(PathBuf::from),
    })
}

/// Heuristic: does this filename stem look like a session UUID?
///
/// Two accepted shapes:
///
/// - Full UUID: 36 chars, 8-4-4-4-12 hex with dashes
///   (e.g. `c3744b8d-5719-4df2-924f-707945438494`).
/// - Short form: 8 hex chars (codex's internal short-id form).
///
/// Anything else (e.g. `README`, `notes`) is rejected so we can give a
/// clearer error than the codex's downstream "session not found".
fn is_uuid_shaped(s: &str) -> bool {
    fn is_hex(c: char) -> bool {
        c.is_ascii_hexdigit()
    }

    // Short form: exactly 8 hex chars.
    if s.len() == 8 && s.chars().all(is_hex) {
        return true;
    }

    // Full form: 8-4-4-4-12 hex with dashes — 36 chars total.
    if s.len() != 36 {
        return false;
    }
    let bytes = s.as_bytes();
    let dash_positions = [8usize, 13, 18, 23];
    for (i, b) in bytes.iter().enumerate() {
        let c = *b as char;
        if dash_positions.contains(&i) {
            if c != '-' {
                return false;
            }
        } else if !is_hex(c) {
            return false;
        }
    }
    true
}

/// Write the deprecation notice to the given writer, followed by a
/// trailing newline. Extracted as a generic helper so tests can inject
/// a `Vec<u8>` and assert routing without wrestling with process-level
/// stderr capture (no `gag` dev-dep needed). Production wires this to
/// `io::stderr()`.
///
/// Load-bearing: the notice MUST go to stderr, not stdout — pipelines
/// like `mx session export | pandoc` would otherwise be corrupted by
/// the warning text appearing in the document body.
pub(crate) fn print_deprecation_notice<W: Write>(w: &mut W) -> io::Result<()> {
    writeln!(w, "{}", DEPRECATION_NOTICE)
}

/// Entry point invoked by `handle_session(SessionCommands::Export {..})`.
///
/// Prints the deprecation notice to stderr, builds the request, and
/// forwards to `codex::export::run`. Returns whatever the export
/// pipeline returns — errors propagate so existing scripts that check
/// the exit code keep working.
pub fn export_session(path: Option<String>, output: Option<String>) -> Result<()> {
    // Stderr (NOT stdout — would corrupt piped output). The
    // `print_deprecation_notice` helper exists so the routing is
    // unit-testable without sniffing process stderr.
    let _ = print_deprecation_notice(&mut io::stderr());
    let request = build_alias_request(path, output)?;
    crate::codex::export::run(request)?;
    Ok(())
}

/// Walk `~/.claude/projects/` and return the most-recent non-agent
/// session JSONL by mtime.
///
/// Kept here (rather than moved into `codex::archive`) because
/// `codex::archive::resolve_session_path` still calls it for the
/// "archive the currently-live session" path. When `src/session.rs`
/// is deleted in a future PR, this helper migrates with it.
///
/// `pub(crate)`: only `codex::archive` consumes this. Integration
/// tests under `tests/` do not touch it. Tightening from `pub`
/// prevents external surface area from accreting around a helper
/// scheduled for migration.
pub(crate) fn find_most_recent_session() -> Result<PathBuf> {
    let projects_dir = crate::paths::claude_projects_dir();

    if !projects_dir.exists() {
        anyhow::bail!("Claude projects directory not found: {:?}", projects_dir);
    }

    let mut sessions = Vec::new();

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

            // Skip agent sub-sessions — the legacy heuristic.
            if let Some(name) = file_path.file_name().and_then(|n| n.to_str())
                && name.starts_with("agent-")
            {
                continue;
            }

            if let Ok(metadata) = file_entry.metadata()
                && let Ok(modified) = metadata.modified()
            {
                sessions.push((file_path, modified));
            }
        }
    }

    if sessions.is_empty() {
        anyhow::bail!("No non-agent session files found in {:?}", projects_dir);
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.1));

    Ok(sessions[0].0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::export::SessionRef;
    use crate::codex::{Format, Selector};

    // ----- DEPRECATION_NOTICE shape -----

    #[test]
    fn print_deprecation_notice_writes_full_text_to_writer() {
        // Load-bearing: proves the production call site routes the
        // deprecation notice through a Write impl rather than, say,
        // println!() (which would land on stdout and corrupt
        // `mx session export | pandoc` style pipelines). Production
        // wires this same helper to `io::stderr()` — see
        // `export_session`. The matching call site is asserted by
        // `export_session_uses_print_deprecation_notice_with_stderr`.
        let mut buf: Vec<u8> = Vec::new();
        print_deprecation_notice(&mut buf).expect("write to Vec cannot fail");
        let s = String::from_utf8(buf).expect("notice is utf8");
        assert!(
            s.contains(DEPRECATION_NOTICE),
            "writer must receive full DEPRECATION_NOTICE; got: {s}"
        );
        assert!(
            s.ends_with('\n'),
            "notice must terminate with a newline so the next line of output isn't glued to it"
        );
    }

    #[test]
    fn export_session_uses_print_deprecation_notice_with_stderr() {
        // Source-text assertion: confirm `export_session` routes
        // through `print_deprecation_notice(&mut io::stderr())`. Prevents
        // a future refactor from accidentally swapping stderr for
        // stdout (which would corrupt piped output) or replacing the
        // helper with a raw println!.
        let src = include_str!("session.rs");
        // Strip the test module so we're not just matching on a comment
        // mentioning the call shape.
        let prod = src.split("#[cfg(test)]").next().unwrap_or(src);
        assert!(
            prod.contains("print_deprecation_notice(&mut io::stderr())"),
            "export_session must call print_deprecation_notice with io::stderr() — \
             stderr routing is load-bearing for piped output integrity"
        );
        assert!(
            !prod.contains("println!(\"{}\", DEPRECATION_NOTICE)"),
            "deprecation notice must NOT go to stdout"
        );
    }

    #[test]
    fn deprecation_notice_mentions_new_command() {
        assert!(
            DEPRECATION_NOTICE.contains("deprecated"),
            "deprecation notice must contain the word 'deprecated': {}",
            DEPRECATION_NOTICE
        );
        assert!(
            DEPRECATION_NOTICE.contains("mx codex export"),
            "deprecation notice must point at the replacement command: {}",
            DEPRECATION_NOTICE
        );
    }

    // ----- build_alias_request routing -----

    #[test]
    fn empty_path_routes_to_latest() {
        let req = build_alias_request(None, None).unwrap();
        assert!(
            matches!(req.selector, Selector::Latest),
            "no positional should route to Selector::Latest, got {:?}",
            req.selector
        );
    }

    #[test]
    fn non_jsonl_non_uuid_path_is_rejected() {
        // Without this guard, `mx session export README.md` would
        // route to Selector::Session("README") and fail downstream
        // with a codex-flavored "session not found" error — confusing
        // because the input was never a plausible session path.
        let err = build_alias_request(Some("README.md".to_string()), None)
            .expect_err("non-jsonl, non-uuid path must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("session JSONL file"),
            "error must explain expected input shape; got: {msg}"
        );
        assert!(
            msg.contains("mx codex export --session"),
            "error must hint at the replacement command; got: {msg}"
        );
    }

    #[test]
    fn uuid_shaped_stem_without_extension_accepted() {
        // The codex sometimes refers to sessions by bare UUID; accept
        // those without a file extension as a courtesy.
        let req = build_alias_request(
            Some("c3744b8d-5719-4df2-924f-707945438494".to_string()),
            None,
        )
        .expect("uuid-shaped stem should pass validation");
        match req.selector {
            Selector::Session(SessionRef(ref id)) => {
                assert_eq!(id, "c3744b8d-5719-4df2-924f-707945438494");
            }
            other => panic!("expected Selector::Session, got {:?}", other),
        }
    }

    #[test]
    fn is_uuid_shaped_accepts_short_and_full_forms() {
        assert!(is_uuid_shaped("aaaaaaaa"));
        assert!(is_uuid_shaped("c3744b8d-5719-4df2-924f-707945438494"));
        assert!(!is_uuid_shaped("README"));
        assert!(!is_uuid_shaped("notes"));
        assert!(!is_uuid_shaped("zzzzzzzz")); // 8 chars but not hex
        assert!(!is_uuid_shaped("c3744b8d_5719_4df2_924f_707945438494")); // wrong separators
    }

    #[test]
    fn jsonl_path_routes_to_session_uuid() {
        let path = "/home/charlie/.claude/projects/-home-charlie-mx/c3744b8d-5719-4df2-924f-707945438494.jsonl";
        let req = build_alias_request(Some(path.to_string()), None).unwrap();
        match req.selector {
            Selector::Session(SessionRef(ref id)) => {
                assert_eq!(id, "c3744b8d-5719-4df2-924f-707945438494");
            }
            other => panic!("expected Selector::Session, got {:?}", other),
        }
    }

    #[test]
    fn alias_forces_archive_first() {
        // Load-bearing: without this, a user who has never run
        // `mx codex archive` would get "no codex data" instead of the
        // legacy "exported your latest session" behavior.
        let req = build_alias_request(None, None).unwrap();
        assert!(req.archive_first, "alias must force archive_first=true");
    }

    #[test]
    fn alias_forces_markdown_format() {
        let req = build_alias_request(None, None).unwrap();
        assert!(matches!(req.format, Format::Markdown));
    }

    #[test]
    fn alias_default_include_is_clean_with_subagents() {
        // The legacy command silently dropped agent transcripts. The
        // new default includes them — documented in DEPRECATION_NOTICE.
        let req = build_alias_request(None, None).unwrap();
        assert!(req.include.subagents);
        assert!(!req.include.tools);
        assert!(!req.include.system_reminders);
    }

    #[test]
    fn output_path_propagates() {
        let req = build_alias_request(None, Some("/tmp/out.md".to_string())).unwrap();
        assert_eq!(
            req.output.as_deref(),
            Some(std::path::Path::new("/tmp/out.md"))
        );
    }

    #[test]
    fn output_path_absent_propagates_as_none() {
        let req = build_alias_request(None, None).unwrap();
        assert!(req.output.is_none());
    }

    // ----- end-to-end: alias produces the same content as direct codex
    // export -----
    //
    // Mirrors the codex::export integration tests (same `write_archive`
    // shape, same env-var locking pattern). We run the alias against a
    // tempdir codex and assert the output matches what
    // `codex::export::run` would have produced for the same selector.

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

    #[test]
    #[serial]
    fn alias_writes_same_shape_as_codex_export() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        let projects = tmp.path().join("claude-projects-sentinel");
        let alias_out = tmp.path().join("alias.md");
        let direct_out = tmp.path().join("direct.md");
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

        // Direct codex export: latest, markdown, default-clean include.
        let direct_req = crate::codex::ExportRequest {
            selector: Selector::Latest,
            format: Format::Markdown,
            include: crate::codex::ExportIncludeSet::default_clean(),
            archive_first: false,
            output: Some(direct_out.clone()),
        };
        let direct_result = crate::codex::export::run(direct_req);

        // Alias: same effective invocation routed through build_alias_request.
        // Construct the request by hand here (rather than calling
        // export_session, which prints to stderr) so the test stays
        // tidy — the shape-equivalence is what matters.
        let mut alias_req =
            build_alias_request(None, Some(alias_out.to_string_lossy().to_string()))
                .expect("build_alias_request");
        // We flip `archive_first` off here because re-archiving a
        // tempdir with no live sources would itself work but adds
        // nondeterminism (timing, fs walks, env-dependent claude
        // projects discovery) to a test that's measuring output
        // equivalence, not archive behavior. The on-by-default
        // invariant is asserted separately by
        // `alias_forces_archive_first`, and PR #270's tests cover the
        // archive_first → run integration end-to-end.
        alias_req.archive_first = false;
        let alias_result = crate::codex::export::run(alias_req);

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

        let _ = direct_result.expect("direct export::run failed");
        let _ = alias_result.expect("alias export::run failed");
        let direct_body = std::fs::read_to_string(&direct_out).unwrap();
        let alias_body = std::fs::read_to_string(&alias_out).unwrap();
        assert_eq!(
            direct_body, alias_body,
            "alias output must equal direct `mx codex export` output"
        );
        assert!(alias_body.contains("Session aaaaaaaa-1111"));
        assert!(alias_body.contains("hello"));
    }

    // ----- find_most_recent_session smoke test (kept from the legacy
    // module — archive depends on this helper) -----

    #[test]
    #[serial]
    fn find_most_recent_session_errors_when_dir_missing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");

        let prev_home = std::env::var("HOME").ok();
        let prev_mx_home = std::env::var("MX_HOME").ok();
        // SAFETY: serial + lock.
        unsafe {
            std::env::set_var("HOME", &nonexistent);
            std::env::remove_var("MX_HOME");
        }
        let result = find_most_recent_session();
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            if let Some(v) = prev_mx_home {
                std::env::set_var("MX_HOME", v);
            }
        }
        assert!(result.is_err(), "missing projects dir must error");
    }
}
