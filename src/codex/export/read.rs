//! Codex-store reader for `mx codex export`.
//!
//! Reads a single archive directory and returns the on-disk artifacts
//! the format emitters render: the session source (jsonl OR
//! conversation.md), every agent JSONL, and the optional sidecars
//! (mcp/, tool-output/, history/).
//!
//! Reads are scoped to `<codex_dir>/<archive_dir>/` exclusively. We do
//! NOT read `~/.claude/projects/` here — that's the architectural
//! invariant. The detection layer (detect.rs) is the only path that
//! touches live Claude data, and it never reads content for rendering.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// What `read_archive` returns: every on-disk artifact the emitters
/// might want to render, with `None` for missing-but-optional pieces.
#[derive(Debug)]
pub struct LoadedArchive {
    pub archive_dir: PathBuf,
    /// Raw `session.jsonl` content, if present.
    pub session_jsonl: Option<String>,
    /// Pre-rendered `conversation.md` if archive was clean-mode.
    pub conversation_md: Option<String>,
    /// Sub-agent JSONLs: `(name, content)` tuples sorted by name.
    pub agents: Vec<(String, String)>,
    /// MCP log file paths under `mcp/` (raw — emitters decide how to
    /// render).
    pub mcp_logs: Vec<PathBuf>,
    /// Tool-output file paths under `tool-output/`.
    pub tool_outputs: Vec<PathBuf>,
    /// Captured history slice content if `history/history.jsonl` exists.
    pub history_jsonl: Option<String>,
}

impl LoadedArchive {
    /// True iff at least one human/assistant content source was loaded.
    pub fn has_transcript(&self) -> bool {
        self.session_jsonl.is_some() || self.conversation_md.is_some()
    }
}

/// Load every artifact under `archive_dir`. Returns `Ok` even if some
/// pieces are missing — the emitter decides what's mandatory.
pub fn read_archive(archive_dir: &Path) -> Result<LoadedArchive> {
    let session_jsonl = read_optional(&archive_dir.join("session.jsonl"))?;
    let conversation_md = read_optional(&archive_dir.join("conversation.md"))?;
    let agents = read_agents(&archive_dir.join("agents"))?;
    let mcp_logs = list_dir_files(&archive_dir.join("mcp"))?;
    let tool_outputs = list_dir_files(&archive_dir.join("tool-output"))?;
    let history_jsonl = read_optional(&archive_dir.join("history").join("history.jsonl"))?;

    Ok(LoadedArchive {
        archive_dir: archive_dir.to_path_buf(),
        session_jsonl,
        conversation_md,
        agents,
        mcp_logs,
        tool_outputs,
        history_jsonl,
    })
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

fn read_agents(agents_dir: &Path) -> Result<Vec<(String, String)>> {
    if !agents_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".jsonl") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        out.push((name, content));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn list_dir_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_empty_archive_has_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = read_archive(tmp.path()).unwrap();
        assert!(!loaded.has_transcript());
        assert!(loaded.agents.is_empty());
        assert!(loaded.mcp_logs.is_empty());
        assert!(loaded.tool_outputs.is_empty());
        assert!(loaded.history_jsonl.is_none());
    }

    #[test]
    fn read_archive_finds_session_and_agents() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("session.jsonl"), "{}\n").unwrap();
        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        fs::write(tmp.path().join("agents/agent-aaaa.jsonl"), "agent-a\n").unwrap();
        fs::write(tmp.path().join("agents/agent-bbbb.jsonl"), "agent-b\n").unwrap();
        // Plus stray non-jsonl file that should be ignored.
        fs::write(tmp.path().join("agents/notes.txt"), "ignored").unwrap();

        let loaded = read_archive(tmp.path()).unwrap();
        assert!(loaded.session_jsonl.is_some());
        assert_eq!(loaded.agents.len(), 2);
        assert_eq!(loaded.agents[0].0, "agent-aaaa.jsonl");
        assert_eq!(loaded.agents[1].0, "agent-bbbb.jsonl");
    }

    #[test]
    fn read_archive_finds_optional_sidecars() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("mcp")).unwrap();
        fs::write(tmp.path().join("mcp/server-1.jsonl"), "log\n").unwrap();
        fs::create_dir_all(tmp.path().join("tool-output")).unwrap();
        fs::write(tmp.path().join("tool-output/abc.output"), "out\n").unwrap();
        fs::create_dir_all(tmp.path().join("history")).unwrap();
        fs::write(
            tmp.path().join("history/history.jsonl"),
            "{\"timestamp\": \"2026-04-29T12:00:00Z\"}\n",
        )
        .unwrap();

        let loaded = read_archive(tmp.path()).unwrap();
        assert_eq!(loaded.mcp_logs.len(), 1);
        assert_eq!(loaded.tool_outputs.len(), 1);
        assert!(loaded.history_jsonl.is_some());
    }

    #[test]
    fn read_archive_prefers_both_when_both_present() {
        // Clean-mode archives may have BOTH conversation.md and session.jsonl
        // — the emitter chooses precedence; the reader exposes both.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("session.jsonl"), "{}\n").unwrap();
        fs::write(tmp.path().join("conversation.md"), "# hi\n").unwrap();
        let loaded = read_archive(tmp.path()).unwrap();
        assert!(loaded.session_jsonl.is_some());
        assert!(loaded.conversation_md.is_some());
    }
}
