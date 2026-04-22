mod archive;
mod images;
mod migrate;
mod read;
mod transcript;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Re-export public API
pub(crate) use archive::save_session;
pub(crate) use migrate::migrate_archives;
pub(crate) use read::{list_sessions, read_session, search_archives};

/// Manifest metadata for an archived session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub session_id: String,
    pub archived_at: DateTime<Utc>,
    pub session_start: DateTime<Utc>,
    pub session_end: DateTime<Utc>,
    pub project_path: Option<String>,
    pub message_count: usize,
    pub agent_count: usize,
    pub agents: Vec<AgentInfo>,
    pub size_bytes: u64,
    pub checksum: String,
    // v2 fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageInfo>>,
    // v3 fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_clean_transcript: Option<bool>,
    // v4 fields - configurable speaker names
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub file: String,
    pub messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub hash: String,
    pub media_type: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_tool_use_id: Option<String>,
}

// Internal shared type used by archive, read, and migrate submodules
#[derive(Debug, Clone)]
pub(crate) struct ArchiveEntry {
    pub(crate) dir_name: String,
    pub(crate) short_id: String,
    pub(crate) incremental: u32,
    pub(crate) manifest: Manifest,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // W4: backwards-compat & edge-case tests
    // ---------------------------------------------------------------------------

    #[test]
    fn manifest_deserialize_without_speaker_names() {
        // Old archives will not have user_name or assistant_name fields.
        // Verify they deserialize as None for backwards compatibility.
        let json = r#"{
            "version": 2,
            "session_id": "abc123",
            "archived_at": "2026-01-01T00:00:00Z",
            "session_start": "2026-01-01T00:00:00Z",
            "session_end": "2026-01-01T00:00:00Z",
            "project_path": null,
            "message_count": 10,
            "agent_count": 0,
            "agents": [],
            "size_bytes": 1024,
            "checksum": "sha256:aaa"
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.user_name, None);
        assert_eq!(manifest.assistant_name, None);
        assert_eq!(manifest.has_clean_transcript, None);
        assert_eq!(manifest.image_count, None);
    }

    // ---------------------------------------------------------------------------
    // archive_source: format-detection regression test
    // ---------------------------------------------------------------------------

    #[test]
    fn archive_source_prefers_clean_md_over_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("conversation.md");
        let jsonl_path = dir.path().join("session.jsonl");

        // Only conversation.md present (clean-mode archive)
        std::fs::write(&md_path, "# Conversation\n").unwrap();
        let result = read::archive_source(dir.path()).unwrap();
        assert_eq!(result, md_path, "should prefer conversation.md");

        // Both present — still prefers conversation.md
        std::fs::write(&jsonl_path, "{}\n").unwrap();
        let result = read::archive_source(dir.path()).unwrap();
        assert_eq!(
            result, md_path,
            "should prefer conversation.md when both exist"
        );

        // Only session.jsonl present (legacy archive)
        std::fs::remove_file(&md_path).unwrap();
        let result = read::archive_source(dir.path()).unwrap();
        assert_eq!(result, jsonl_path, "should fall back to session.jsonl");

        // Neither present — returns None
        std::fs::remove_file(&jsonl_path).unwrap();
        assert!(
            read::archive_source(dir.path()).is_none(),
            "should return None when no transcript exists"
        );
    }

    #[test]
    fn archive_source_real_archives_are_searchable() {
        // Cross-reference rail: at least one real archive must be in the canonical
        // format that archive_source can find.  This is the test that would have
        // caught the asymmetric migration at CI time — the check that clean-mode
        // archives are actually visible to the search path.
        let codex_dir = match archive::get_codex_dir() {
            Ok(d) => d,
            Err(_) => return, // codex dir not configured in this environment — skip
        };
        if !codex_dir.exists() {
            return; // no archives yet — skip rather than fail
        }
        let archives = match archive::collect_archives(&codex_dir) {
            Ok(a) => a,
            Err(_) => return,
        };
        if archives.is_empty() {
            return; // nothing to check
        }
        let searchable = archives.iter().filter(|a| {
            let archive_dir = codex_dir.join(&a.dir_name);
            read::archive_source(&archive_dir).is_some()
        });
        assert!(
            searchable.count() > 0,
            "no archives are searchable — archive_source found neither \
             conversation.md nor session.jsonl in any of the {} archive(s) \
             under {:?}. search_archives would return zero results.",
            archives.len(),
            codex_dir
        );
    }
}
