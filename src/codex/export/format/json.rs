//! Structured-JSON emitter for `mx codex export`.
//!
//! Schema (top-level object):
//!
//! ```jsonc
//! {
//!   "manifest":   { ...the codex manifest verbatim... },
//!   "events":     [ ...parsed session events, filtered by include flags... ],
//!   "agents":     [ { "name": "...", "events": [...] }, ... ],
//!   "sidecars": {
//!     "mcp_logs":     [ { "name": "...", "content": "..." }, ... ],
//!     "tool_outputs": [ { "name": "...", "content": "..." }, ... ],
//!     "history":      [ ...lines as JSON values, when parseable... ]
//!   }
//! }
//! ```
//!
//! Events keep their raw shape from the JSONL — we don't transform them
//! beyond filtering based on the include flags. This is the form tool
//! consumers want; humans should pick the markdown emitter.

use anyhow::Result;
use serde_json::{Value, json};

use crate::codex::Manifest;
use crate::codex::export::include::ExportIncludeSet;
use crate::codex::export::read::LoadedArchive;

/// Render a loaded archive to a JSON string.
pub fn render(
    archive: &LoadedArchive,
    manifest: &Manifest,
    include: &ExportIncludeSet,
) -> Result<String> {
    let manifest_json = serde_json::to_value(manifest)?;
    let events = filter_events(&archive.session_jsonl, include);
    let agents = render_agents(archive, include);
    let sidecars = render_sidecars(archive, include);

    let doc = json!({
        "manifest": manifest_json,
        "events": events,
        "agents": agents,
        "sidecars": sidecars,
    });
    Ok(serde_json::to_string_pretty(&doc)?)
}

fn filter_events(content: &Option<String>, include: &ExportIncludeSet) -> Vec<Value> {
    let mut out = Vec::new();
    let content = match content {
        Some(c) => c,
        None => return out,
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Strip `tool_use` blocks from assistant content if tools are off.
        if !include.tools
            && value["type"].as_str() == Some("assistant")
            && let Some(blocks) = value["message"]["content"].as_array()
        {
            let kept: Vec<Value> = blocks
                .iter()
                .filter(|b| b["type"].as_str() != Some("tool_use"))
                .cloned()
                .collect();
            value["message"]["content"] = Value::Array(kept);
        }
        // Strip `<system-reminder>...</system-reminder>` from text fields
        // if system_reminders are off.
        if !include.system_reminders {
            scrub_system_reminders(&mut value);
        }
        out.push(value);
    }
    out
}

fn scrub_system_reminders(value: &mut Value) {
    use crate::codex::export::format::markdown::system_reminder_re_for_export;
    match value {
        Value::String(s) => {
            *s = system_reminder_re_for_export()
                .replace_all(s, "")
                .to_string();
        }
        Value::Array(arr) => {
            for v in arr {
                scrub_system_reminders(v);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                scrub_system_reminders(v);
            }
        }
        _ => {}
    }
}

fn render_agents(archive: &LoadedArchive, include: &ExportIncludeSet) -> Vec<Value> {
    if !include.subagents {
        return Vec::new();
    }
    archive
        .agents
        .iter()
        .map(|(name, content)| {
            let events = filter_events(&Some(content.clone()), include);
            json!({ "name": name, "events": events })
        })
        .collect()
}

fn render_sidecars(archive: &LoadedArchive, include: &ExportIncludeSet) -> Value {
    let mcp = if include.mcp {
        archive
            .mcp_logs
            .iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("(unnamed)")
                    .to_string();
                let content = std::fs::read_to_string(p).unwrap_or_default();
                json!({ "name": name, "content": content })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let tool_outputs = if include.tool_output {
        archive
            .tool_outputs
            .iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("(unnamed)")
                    .to_string();
                let content = std::fs::read_to_string(p).unwrap_or_default();
                json!({ "name": name, "content": content })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let history = if include.history
        && let Some(content) = &archive.history_jsonl
    {
        content
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    json!({
        "mcp_logs": mcp,
        "tool_outputs": tool_outputs,
        "history": history,
    })
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
            session_id: "test".to_string(),
            archived_at: Utc::now(),
            session_start: "2026-04-29T10:00:00Z".parse().unwrap(),
            session_end: "2026-04-29T10:30:00Z".parse().unwrap(),
            project_path: Some("/home/charlie/work/mx".to_string()),
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
        }
    }

    fn empty_archive(jsonl: &str) -> LoadedArchive {
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
    fn render_top_level_keys() {
        let m = fixture_manifest();
        let arc = empty_archive("");
        let out = render(&arc, &m, &ExportIncludeSet::default_clean()).unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert!(parsed.get("manifest").is_some());
        assert!(parsed.get("events").is_some());
        assert!(parsed.get("agents").is_some());
        assert!(parsed.get("sidecars").is_some());
    }

    #[test]
    fn render_strips_tool_use_when_tools_disabled() {
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Bash"},{"type":"text","text":"ok"}]}}"#;
        let m = fixture_manifest();
        let arc = empty_archive(jsonl);
        let out = render(&arc, &m, &ExportIncludeSet::default_clean()).unwrap();
        assert!(!out.contains("tool_use"));
        assert!(out.contains("\"ok\""));
    }

    #[test]
    fn render_keeps_tool_use_when_tools_enabled() {
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Bash"},{"type":"text","text":"ok"}]}}"#;
        let m = fixture_manifest();
        let arc = empty_archive(jsonl);
        let mut inc = ExportIncludeSet::default_clean();
        inc.tools = true;
        let out = render(&arc, &m, &inc).unwrap();
        assert!(out.contains("tool_use"));
        assert!(out.contains("Bash"));
    }
}
