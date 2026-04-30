mod archive;
mod images;
pub mod index;
mod migrate;
mod read;
mod transcript;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Re-export public API
pub(crate) use archive::{IncludeSet, save_session};
pub(crate) use migrate::migrate_archives;
pub(crate) use read::{list_sessions, read_session, search_archives};

/// Current manifest write version. Bumped from `2` to `5` in the codex
/// foundation PR: v3 (`has_clean_transcript`) and v4 (`user_name`,
/// `assistant_name`) had been declared as latent optional fields without a
/// matching writer bump, and v5 adds the source-breakdown / sidecar-count
/// fields needed by the unified ingestion flow. All new fields are
/// `Option`, so v2/v3/v4 archives still deserialize cleanly.
pub(crate) const MANIFEST_WRITE_VERSION: u32 = 5;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageInfo>>,
    // v3 fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_clean_transcript: Option<bool>,
    // v4 fields - configurable speaker names
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_name: Option<String>,
    // v5 fields - new sidecar counts and per-source byte breakdown.
    // Defaulted to None in this PR; populated by PR 2 when archive starts
    // writing the new sidecars (mcp/, tool-output/, history/).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_log_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_breakdown: Option<SourceBreakdown>,
}

/// Per-sidecar byte counts. Captured at archive time so `mx codex export`
/// and downstream tooling can reason about disk usage without re-stat-ing
/// every file.
///
/// All fields default to `0` rather than `Option<u64>` because the
/// breakdown as a whole is `Option`-wrapped on the manifest — a present
/// `SourceBreakdown` means "we measured everything," and an absent sidecar
/// is naturally represented by a zero count.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceBreakdown {
    #[serde(default)]
    pub session_jsonl_bytes: u64,
    #[serde(default)]
    pub agents_bytes: u64,
    #[serde(default)]
    pub images_bytes: u64,
    #[serde(default)]
    pub mcp_bytes: u64,
    #[serde(default)]
    pub tool_output_bytes: u64,
    #[serde(default)]
    pub history_bytes: u64,
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
        // v5 additions must also default to None for back-compat.
        assert_eq!(manifest.tool_output_count, None);
        assert_eq!(manifest.mcp_log_count, None);
        assert_eq!(manifest.history_lines, None);
        assert_eq!(manifest.source_breakdown, None);
    }

    #[test]
    fn manifest_v2_round_trips_through_v5() {
        // A v2 manifest from the wild deserializes into the v5 struct, and
        // re-serializing it as v5 (after a metadata-only version bump)
        // round-trips back into an equivalent struct. The v5 fields stay
        // None throughout — that's the whole point of "metadata-only bump."
        let v2_json = r#"{
            "version": 2,
            "session_id": "abc123",
            "archived_at": "2026-01-01T00:00:00Z",
            "session_start": "2026-01-01T00:00:00Z",
            "session_end": "2026-01-01T00:00:00Z",
            "project_path": "/home/test/project",
            "message_count": 42,
            "agent_count": 1,
            "agents": [{"id": "ag-1", "file": "agents/agent-1.jsonl", "messages": 7}],
            "size_bytes": 99999,
            "checksum": "sha256:deadbeef",
            "image_count": 3,
            "images": [{"hash": "h1", "media_type": "image/png", "size_bytes": 100}]
        }"#;

        let mut manifest: Manifest = serde_json::from_str(v2_json).unwrap();
        assert_eq!(manifest.version, 2);
        assert_eq!(manifest.image_count, Some(3));
        assert_eq!(manifest.tool_output_count, None);
        assert_eq!(manifest.source_breakdown, None);

        // Simulate the migration: bump version, leave new fields None.
        manifest.version = MANIFEST_WRITE_VERSION;

        let v5_json = serde_json::to_string_pretty(&manifest).unwrap();
        let round_tripped: Manifest = serde_json::from_str(&v5_json).unwrap();
        assert_eq!(round_tripped.version, MANIFEST_WRITE_VERSION);
        assert_eq!(round_tripped.session_id, "abc123");
        assert_eq!(round_tripped.message_count, 42);
        assert_eq!(round_tripped.image_count, Some(3));
        assert_eq!(round_tripped.tool_output_count, None);
        assert_eq!(round_tripped.mcp_log_count, None);
        assert_eq!(round_tripped.history_lines, None);
        assert_eq!(round_tripped.source_breakdown, None);

        // The skip_serializing_if guards mean the serialized v5 of a
        // metadata-only bump should not contain the new field names at all.
        assert!(!v5_json.contains("tool_output_count"));
        assert!(!v5_json.contains("mcp_log_count"));
        assert!(!v5_json.contains("history_lines"));
        assert!(!v5_json.contains("source_breakdown"));
    }

    #[test]
    fn manifest_v5_with_new_fields_round_trips() {
        // When the new fields *are* populated (PR 2 territory), they
        // serialize and deserialize symmetrically.
        let breakdown = SourceBreakdown {
            session_jsonl_bytes: 1024,
            agents_bytes: 512,
            images_bytes: 2048,
            mcp_bytes: 256,
            tool_output_bytes: 128,
            history_bytes: 64,
        };
        let manifest = Manifest {
            version: MANIFEST_WRITE_VERSION,
            session_id: "v5-test".to_string(),
            archived_at: "2026-04-29T00:00:00Z".parse().unwrap(),
            session_start: "2026-04-29T00:00:00Z".parse().unwrap(),
            session_end: "2026-04-29T01:00:00Z".parse().unwrap(),
            project_path: None,
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
            tool_output_count: Some(5),
            mcp_log_count: Some(2),
            history_lines: Some(17),
            source_breakdown: Some(breakdown.clone()),
        };
        let s = serde_json::to_string(&manifest).unwrap();
        let back: Manifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.tool_output_count, Some(5));
        assert_eq!(back.mcp_log_count, Some(2));
        assert_eq!(back.history_lines, Some(17));
        assert_eq!(back.source_breakdown, Some(breakdown));
    }

    #[test]
    fn manifest_write_version_is_five() {
        // Pinning constant matches the design doc.
        assert_eq!(MANIFEST_WRITE_VERSION, 5);
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
