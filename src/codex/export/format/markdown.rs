//! Markdown emitter for `mx codex export`.
//!
//! Two rendering paths, in priority order:
//!
//! 1. If the archive has a `conversation.md` (i.e. it was archived in
//!    clean mode), prefer that for the human/assistant prose. Apply the
//!    include filters as best we can — `system_reminders` and `tools`
//!    can't actually be re-introduced (clean mode already stripped
//!    them), but we still honour the filters for the optional sidecars
//!    appended below.
//! 2. Otherwise render from the raw `session.jsonl`. Walk every event
//!    line and:
//!    - skip `<system-reminder>` blocks unless `include.system_reminders`
//!    - skip `tool_use` blocks unless `include.tools`
//!    - render human / assistant turns as Markdown
//!    - inline subagent transcripts at the parent's Task-tool boundary
//!      if `include.subagents`
//!
//! After the prose, optional sidecars are appended as separate sections
//! gated on the matching include flag.

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

use crate::codex::Manifest;
use crate::codex::export::include::ExportIncludeSet;
use crate::codex::export::read::LoadedArchive;

static SYSTEM_REMINDER_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled pattern for stripping `<system-reminder>...</system-reminder>`
/// blocks injected by the platform.
///
/// **Known limitation (v1, by design).** This is a textual heuristic: any
/// occurrence of the byte sequence `<system-reminder>...</system-reminder>`
/// in a string field is removed, regardless of who wrote it. A user
/// message that legitimately quotes that sequence — e.g. an excerpt of a
/// meta-conversation about reminders — will be incorrectly stripped along
/// with the platform-injected blocks. Acceptable for v1: the failure mode
/// is a redaction, not data leakage, and the false-positive rate on real
/// transcripts is effectively zero. Future work could anchor stripping to
/// the platform-injection envelope (a stable discriminator on the message
/// shape rather than a substring match) once the platform exposes one.
/// Tracked as TODO(#254-followup).
fn system_reminder_re() -> &'static Regex {
    SYSTEM_REMINDER_RE.get_or_init(|| {
        Regex::new(r"(?s)<system-reminder>.*?</system-reminder>")
            .expect("system-reminder regex must compile")
    })
}

/// Cross-emitter accessor: the JSON emitter scrubs `<system-reminder>`
/// blocks from string fields too, so it borrows the same compiled regex.
///
/// Inherits the textual-heuristic limitation documented on
/// [`system_reminder_re`]. The JSON walker recurses into every string
/// value in the document, so the same false-positive applies anywhere a
/// quoted `<system-reminder>...</system-reminder>` appears in legitimate
/// content.
pub(crate) fn system_reminder_re_for_export() -> &'static Regex {
    system_reminder_re()
}

/// Render a loaded archive to a single Markdown document.
pub fn render(
    archive: &LoadedArchive,
    manifest: &Manifest,
    include: &ExportIncludeSet,
) -> Result<String> {
    let mut out = String::new();

    // Header: enough metadata to be self-describing without dumping the
    // full manifest.
    out.push_str(&format!("# Session {}\n\n", manifest.session_id));
    out.push_str(&format!(
        "- **Started:** {}\n",
        manifest.session_start.to_rfc3339()
    ));
    out.push_str(&format!(
        "- **Ended:** {}\n",
        manifest.session_end.to_rfc3339()
    ));
    if let Some(p) = &manifest.project_path {
        out.push_str(&format!("- **Project:** `{}`\n", p));
    }
    out.push_str(&format!(
        "- **Messages:** {} (across {} agent(s))\n",
        manifest.message_count, manifest.agent_count
    ));
    out.push('\n');

    // -- Conversation body --

    if let Some(md) = &archive.conversation_md {
        // Clean-mode archive: re-use the on-disk markdown. The include
        // filters for `tools` / `system-reminders` are advisory here —
        // the clean transcript stripped them at archive time, so there's
        // nothing left to re-introduce. Document this in the rendered
        // output so consumers know the filters are partially load-bearing.
        out.push_str("## Conversation\n\n");
        if include.system_reminders || include.tools {
            out.push_str(
                "_Note: this archive was captured in clean mode. \
                 `tools` / `system-reminders` filters cannot reintroduce \
                 content that was stripped at archive time; rerun with a \
                 full-mode archive if you need them._\n\n",
            );
        }
        out.push_str(md);
        if !md.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    } else if let Some(jsonl) = &archive.session_jsonl {
        out.push_str("## Conversation\n\n");
        out.push_str(&render_jsonl_body(jsonl, include)?);

        // Inline sub-agent transcripts after the parent prose if requested.
        if include.subagents {
            for (name, content) in &archive.agents {
                let agent_md = render_jsonl_body(content, include)?;
                if !agent_md.trim().is_empty() {
                    out.push_str("\n---\n\n");
                    out.push_str(&format!("## Agent: {}\n\n", name));
                    out.push_str(&agent_md);
                }
            }
        }
    } else {
        out.push_str("## Conversation\n\n_(no transcript available)_\n\n");
    }

    // -- Optional sidecar sections --

    if include.mcp && !archive.mcp_logs.is_empty() {
        out.push_str("\n## MCP Logs\n\n");
        for path in &archive.mcp_logs {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(unnamed)");
            out.push_str(&format!("### {}\n\n", name));
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    out.push_str("```jsonl\n");
                    out.push_str(content.trim_end_matches('\n'));
                    out.push_str("\n```\n\n");
                }
                Err(e) => {
                    out.push_str(&format!("_(failed to read: {})_\n\n", e));
                }
            }
        }
    }

    if include.tool_output && !archive.tool_outputs.is_empty() {
        out.push_str("\n## Tool Output\n\n");
        for path in &archive.tool_outputs {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(unnamed)");
            out.push_str(&format!("### {}\n\n", name));
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    out.push_str("```\n");
                    out.push_str(content.trim_end_matches('\n'));
                    out.push_str("\n```\n\n");
                }
                Err(e) => {
                    out.push_str(&format!("_(failed to read: {})_\n\n", e));
                }
            }
        }
    }

    if include.history
        && let Some(history) = &archive.history_jsonl
        && !history.trim().is_empty()
    {
        out.push_str("\n## History Slice\n\n");
        out.push_str("```jsonl\n");
        out.push_str(history.trim_end_matches('\n'));
        out.push_str("\n```\n");
    }

    Ok(out)
}

/// Render the body (no header, no sidecars) of a session.jsonl according
/// to the include flags. Used for both the parent and (recursively) the
/// inlined subagent transcripts.
fn render_jsonl_body(content: &str, include: &ExportIncludeSet) -> Result<String> {
    let mut out = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_type = match msg["type"].as_str() {
            Some(t) => t,
            None => continue,
        };

        match msg_type {
            "user" => {
                let content_node = &msg["message"]["content"];
                if let Some(text) = content_node.as_str() {
                    let processed = if include.system_reminders {
                        text.to_string()
                    } else {
                        system_reminder_re().replace_all(text, "").to_string()
                    };
                    let trimmed = processed.trim();
                    if !trimmed.is_empty() {
                        out.push_str(&format!("**User:** {}\n\n", trimmed));
                    }
                }
                // Array content (tool results) — skip in markdown render
                // unless tools is on. The block-level rendering of tool
                // results lives in the tools branch under "assistant".
            }
            "assistant" => {
                if let Some(blocks) = msg["message"]["content"].as_array() {
                    let mut text_parts = Vec::new();
                    let mut tool_parts = Vec::new();
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(text) = block["text"].as_str() {
                                    let processed = if include.system_reminders {
                                        text.to_string()
                                    } else {
                                        system_reminder_re().replace_all(text, "").to_string()
                                    };
                                    let trimmed = processed.trim();
                                    if !trimmed.is_empty() {
                                        text_parts.push(trimmed.to_string());
                                    }
                                }
                            }
                            Some("tool_use") if include.tools => {
                                let name = block["name"].as_str().unwrap_or("(tool)");
                                let id = block["id"].as_str().unwrap_or("");
                                let input = serde_json::to_string_pretty(&block["input"])
                                    .unwrap_or_else(|_| "{}".to_string());
                                tool_parts.push(format!(
                                    "**Tool use:** `{}` (id: `{}`)\n\n```json\n{}\n```",
                                    name, id, input
                                ));
                            }
                            _ => {}
                        }
                    }
                    let prose = text_parts.join("\n\n");
                    if !prose.is_empty() {
                        out.push_str(&format!("**Assistant:** {}\n\n", prose));
                    }
                    for tool in tool_parts {
                        out.push_str(&tool);
                        out.push_str("\n\n");
                    }
                }
            }
            _ => {}
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::MANIFEST_WRITE_VERSION;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fixture_manifest() -> Manifest {
        Manifest {
            version: MANIFEST_WRITE_VERSION,
            session_id: "test-session-id".to_string(),
            archived_at: Utc::now(),
            session_start: "2026-04-29T10:00:00Z".parse().unwrap(),
            session_end: "2026-04-29T10:30:00Z".parse().unwrap(),
            project_path: Some("/home/charlie/work/mx".to_string()),
            message_count: 4,
            agent_count: 1,
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
        }
    }

    fn fixture_jsonl() -> &'static str {
        concat!(
            r#"{"type":"user","message":{"content":"hello there"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi back"}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<system-reminder>secret</system-reminder>visible"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}},{"type":"text","text":"running"}]}}"#,
            "\n",
        )
    }

    fn empty_archive_with_jsonl(jsonl: &str) -> LoadedArchive {
        LoadedArchive {
            archive_dir: PathBuf::from("/nonexistent"),
            session_jsonl: Some(jsonl.to_string()),
            conversation_md: None,
            agents: vec![],
            mcp_logs: vec![],
            tool_outputs: vec![],
            history_jsonl: None,
        }
    }

    #[test]
    fn render_default_clean_strips_system_reminders_and_tools() {
        let m = fixture_manifest();
        let arc = empty_archive_with_jsonl(fixture_jsonl());
        let out = render(&arc, &m, &ExportIncludeSet::default_clean()).unwrap();
        assert!(out.contains("**User:** hello there"));
        assert!(out.contains("**Assistant:** hi back"));
        assert!(out.contains("visible"));
        assert!(!out.contains("secret"), "system reminder leaked");
        assert!(!out.contains("Tool use"), "tool_use leaked");
    }

    #[test]
    fn render_with_tools_shows_tool_use_block() {
        let m = fixture_manifest();
        let arc = empty_archive_with_jsonl(fixture_jsonl());
        let mut inc = ExportIncludeSet::default_clean();
        inc.tools = true;
        let out = render(&arc, &m, &inc).unwrap();
        assert!(out.contains("Tool use"));
        assert!(out.contains("Bash"));
        assert!(out.contains("toolu_1"));
    }

    #[test]
    fn render_with_system_reminders_keeps_them() {
        let m = fixture_manifest();
        let arc = empty_archive_with_jsonl(fixture_jsonl());
        let mut inc = ExportIncludeSet::default_clean();
        inc.system_reminders = true;
        let out = render(&arc, &m, &inc).unwrap();
        assert!(out.contains("secret"));
    }

    /// W2 / TODO(#254-followup): the `<system-reminder>` regex is a
    /// textual heuristic and will strip the byte sequence even from
    /// legitimately-quoted user prose (e.g. a meta-conversation excerpt).
    /// This test documents the v1 limitation by asserting the
    /// known-incorrect behavior: a user message that quotes the pattern
    /// IS stripped. When future work anchors stripping to the
    /// platform-injection envelope, this assertion will need to flip
    /// (the prose should survive) and that flip is the signal that the
    /// limitation has been lifted.
    #[test]
    fn render_xfail_strips_legitimately_quoted_system_reminder() {
        // The user is meta-quoting what a reminder block looks like;
        // ideally this prose would be preserved.
        let jsonl = concat!(
            r#"{"type":"user","message":{"content":"For context, the platform sometimes injects <system-reminder>do X</system-reminder> blocks — please ignore them."}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"understood"}]}}"#,
            "\n",
        );
        let m = fixture_manifest();
        let arc = empty_archive_with_jsonl(jsonl);
        let inc = ExportIncludeSet::default_clean(); // system_reminders=false
        let out = render(&arc, &m, &inc).unwrap();
        // XFAIL: the regex strips the quoted block from the user prose.
        // When this changes (correctly preserving the user's quote), the
        // assertions below should flip — that's the signal to revisit
        // the doc comment on `system_reminder_re`.
        assert!(
            !out.contains("do X"),
            "v1 textual heuristic strips the quoted reminder; if this assertion \
             starts failing, the limitation has been lifted — update the doc \
             comment on system_reminder_re()"
        );
        // Surrounding prose is preserved (the regex is non-greedy and
        // only eats the matched span).
        assert!(out.contains("the platform sometimes injects"));
        assert!(out.contains("please ignore them"));
    }

    #[test]
    fn render_inlines_subagents_when_enabled() {
        let m = fixture_manifest();
        let agent_jsonl = concat!(
            r#"{"type":"user","message":{"content":"agent prompt"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"agent answer"}]}}"#,
            "\n",
        );
        let mut arc = empty_archive_with_jsonl(fixture_jsonl());
        arc.agents = vec![("agent-aaaa.jsonl".to_string(), agent_jsonl.to_string())];
        let inc = ExportIncludeSet::default_clean();
        let out = render(&arc, &m, &inc).unwrap();
        assert!(out.contains("## Agent: agent-aaaa.jsonl"));
        assert!(out.contains("agent answer"));
    }

    #[test]
    fn render_skips_subagents_when_disabled() {
        let m = fixture_manifest();
        let agent_jsonl =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"agent answer"}]}}"#;
        let mut arc = empty_archive_with_jsonl(fixture_jsonl());
        arc.agents = vec![("agent-aaaa.jsonl".to_string(), agent_jsonl.to_string())];
        let inc = ExportIncludeSet::none();
        let out = render(&arc, &m, &inc).unwrap();
        assert!(!out.contains("## Agent"));
        assert!(!out.contains("agent answer"));
    }

    #[test]
    fn render_appends_history_section_when_enabled() {
        let m = fixture_manifest();
        let mut arc = empty_archive_with_jsonl(fixture_jsonl());
        arc.history_jsonl = Some("{\"timestamp\": \"2026-04-29T10:15:00Z\"}\n".to_string());
        let mut inc = ExportIncludeSet::default_clean();
        inc.history = true;
        let out = render(&arc, &m, &inc).unwrap();
        assert!(out.contains("## History Slice"));
    }

    #[test]
    fn render_uses_conversation_md_when_present() {
        let m = fixture_manifest();
        let arc = LoadedArchive {
            archive_dir: PathBuf::from("/nonexistent"),
            session_jsonl: None,
            conversation_md: Some("**Charlie:** prebaked\n".to_string()),
            agents: vec![],
            mcp_logs: vec![],
            tool_outputs: vec![],
            history_jsonl: None,
        };
        let out = render(&arc, &m, &ExportIncludeSet::default_clean()).unwrap();
        assert!(out.contains("prebaked"));
    }
}
