use base_d::{DictionaryRegistry, HashAlgorithm, encode, hash};
use serde::{Deserialize, Serialize};

/// A knowledge entry from Zion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub category_id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub applicability: Vec<String>,
    #[serde(default)]
    pub source_project_id: Option<String>,
    #[serde(default)]
    pub source_agent_id: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,

    // Provenance metadata - tracks where knowledge came from
    /// Source type: manual, ram, cache, agent_session
    #[serde(default)]
    pub source_type_id: Option<String>,
    /// Entry type: primary (original), summary, synthesis
    #[serde(default)]
    pub entry_type_id: Option<String>,
    /// Session ID if absorbed from RAM
    #[serde(default)]
    pub session_id: Option<String>,
    /// Ephemeral hint - session-based knowledge that may be pruned
    #[serde(default)]
    pub ephemeral: bool,
    /// Content type: text, code, config, data, binary
    #[serde(default)]
    pub content_type_id: Option<String>,
    /// Owner of the entry (if private)
    #[serde(default)]
    pub owner: Option<String>,
    /// Visibility: public or private
    #[serde(default = "default_visibility")]
    pub visibility: String,

    // Resonance fields - for wake-up cascade
    #[serde(default)]
    pub resonance: i32, // 1-10 (with overflow for transcendent)

    #[serde(default)]
    pub resonance_type: Option<String>, // foundational, transformative, relational, operational, ephemeral

    #[serde(default)]
    pub last_activated: Option<String>, // RFC3339 timestamp

    #[serde(default)]
    pub activation_count: i32,

    #[serde(default = "default_decay_rate")]
    pub decay_rate: f64, // 0.0-1.0, some memories fade, some don't

    #[serde(default)]
    pub anchors: Vec<String>, // IDs of related blooms this connects to

    // Issue #72: Multiple wake phrases
    #[serde(default)]
    pub wake_phrases: Vec<String>, // Multiple phrases for ritual variety

    // Issue #73: Custom wake order
    #[serde(default)]
    pub wake_order: Option<i32>, // Custom wake sequence (lower = earlier)

    // DEPRECATED - kept for backward compatibility during migration
    #[serde(default)]
    pub wake_phrase: Option<String>, // Verification phrase for memory rituals

    // Vector embeddings (PR #89)
    #[serde(default)]
    pub embedding: Option<Vec<f32>>, // 768-dim vector (BGE-Base-EN-v1.5)
    #[serde(default)]
    pub embedding_model: Option<String>, // Model ID that generated the embedding
    #[serde(default)]
    pub embedded_at: Option<String>, // RFC3339 timestamp when embedded

    // Stele encoding format (Issue #122)
    #[serde(default = "default_format")]
    pub format: String, // markdown (default), json, stele:markdown, stele:ascii, stele:light, stele:full

    // Computed decay value (effective_resonance = resonance * decay factor).
    // None when decay hasn't been computed yet. Use this for resonance-sorted display;
    // raw `resonance` does not account for age.
    #[serde(default)]
    pub effective_resonance: Option<f64>,
}

fn default_format() -> String {
    "markdown".to_string()
}

fn default_visibility() -> String {
    "public".to_string()
}

fn default_decay_rate() -> f64 {
    0.0
}

impl KnowledgeEntry {
    /// Returns active wake phrases, preferring wake_phrases over deprecated wake_phrase.
    pub fn active_wake_phrases(&self) -> Vec<&str> {
        if !self.wake_phrases.is_empty() {
            self.wake_phrases.iter().map(|s| s.as_str()).collect()
        } else {
            self.wake_phrase.as_deref().into_iter().collect()
        }
    }

    /// Returns whether this entry has any wake phrase set.
    pub fn has_any_wake_phrase(&self) -> bool {
        !self.wake_phrases.is_empty() || self.wake_phrase.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// Construct text suitable for embedding generation
    ///
    /// Combines title, summary/body, and tags into a single string
    /// optimized for semantic embedding models.
    pub fn embedding_text(&self) -> String {
        let mut parts = vec![self.title.clone()];

        if let Some(summary) = &self.summary {
            parts.push(summary.clone());
        } else if let Some(body) = &self.body {
            // Truncate body to avoid overwhelming the embedding model
            parts.push(body.chars().take(2000).collect());
        }

        if !self.tags.is_empty() {
            parts.push(format!("Tags: {}", self.tags.join(", ")));
        }

        parts.join("\n\n")
    }

    /// Normalize content for comparison (thread matching, etc.)
    ///
    /// Strips whitespace, lowercases, and removes punctuation variations
    /// to enable fuzzy content matching.
    pub fn normalize_content(content: &str) -> String {
        content
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Extract the "state" field from the summary JSON if present
    ///
    /// Many fact types store state in their summary field as JSON.
    /// This helper extracts it safely without duplicating the parsing logic.
    pub fn get_summary_state(&self) -> Option<String> {
        self.summary
            .as_ref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("state").and_then(|s| s.as_str()).map(String::from))
    }

    /// Generate a hash-based ID from path and title
    pub fn generate_id(path: &str, title: &str) -> String {
        let input = format!("{}:{}", path, title);
        let hex = Self::blake3_hex(input.as_bytes());
        format!("kn-{}", &hex[..8])
    }

    /// Compute content hash for change detection
    pub fn compute_hash(content: &str) -> String {
        Self::blake3_hex(content.as_bytes())
    }

    /// Hash data with blake3 and encode as lowercase hex
    fn blake3_hex(data: &[u8]) -> String {
        let hash_bytes = hash(data, HashAlgorithm::Blake3);
        let registry = DictionaryRegistry::load_default().expect("base-d dictionaries");
        let dict = registry.dictionary("base16").expect("base16 dictionary");
        encode(&hash_bytes, &dict).to_lowercase()
    }

    // NOTE: `KnowledgeEntry::from_markdown` was removed alongside
    // `mx memory rebuild`. Markdown ingest will return as a follow-up; the
    // walker logic and YAML frontmatter parser have been deleted.
    // TODO(legacy-state-cleanup): nothing further here -- listed for grep.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_id() {
        let id = KnowledgeEntry::generate_id("pattern/test.md", "Test Pattern");
        assert!(id.starts_with("kn-"));
        assert_eq!(id.len(), 11); // "kn-" + 8 hex chars
    }

    #[test]
    fn test_normalize_content() {
        // Basic whitespace normalization
        assert_eq!(
            KnowledgeEntry::normalize_content("  hello   world  "),
            "hello world"
        );

        // Case insensitive
        assert_eq!(
            KnowledgeEntry::normalize_content("Hello World"),
            "hello world"
        );

        // Multi-line collapsed
        assert_eq!(
            KnowledgeEntry::normalize_content("hello\n  world\n  test"),
            "hello world test"
        );

        // Tab handling
        assert_eq!(
            KnowledgeEntry::normalize_content("hello\tworld"),
            "hello world"
        );
    }

    #[test]
    fn test_embedding_text() {
        let entry = KnowledgeEntry {
            id: "kn-test".to_string(),
            title: "Test Entry".to_string(),
            body: Some("This is the body content.".to_string()),
            summary: None,
            tags: vec!["rust".to_string(), "test".to_string()],
            category_id: "technique".to_string(),
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            created_at: None,
            updated_at: None,
            content_hash: None,
            source_type_id: None,
            entry_type_id: None,
            session_id: None,
            ephemeral: false,
            content_type_id: None,
            owner: None,
            visibility: "public".to_string(),
            resonance: 0,
            resonance_type: None,
            last_activated: None,
            activation_count: 0,
            decay_rate: 0.0,
            anchors: vec![],
            wake_phrases: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            format: "markdown".to_string(),
            effective_resonance: None,
        };

        let text = entry.embedding_text();
        assert!(text.contains("Test Entry"));
        assert!(text.contains("This is the body content."));
        assert!(text.contains("Tags: rust, test"));
    }

    #[test]
    fn test_embedding_text_with_summary() {
        let entry = KnowledgeEntry {
            id: "kn-test".to_string(),
            title: "Test Entry".to_string(),
            body: Some("Long body that should be ignored when summary exists.".to_string()),
            summary: Some("Short summary".to_string()),
            tags: vec![],
            category_id: "technique".to_string(),
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            created_at: None,
            updated_at: None,
            content_hash: None,
            source_type_id: None,
            entry_type_id: None,
            session_id: None,
            ephemeral: false,
            content_type_id: None,
            owner: None,
            visibility: "public".to_string(),
            resonance: 0,
            resonance_type: None,
            last_activated: None,
            activation_count: 0,
            decay_rate: 0.0,
            anchors: vec![],
            wake_phrases: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            format: "markdown".to_string(),
            effective_resonance: None,
        };

        let text = entry.embedding_text();
        assert!(text.contains("Test Entry"));
        assert!(text.contains("Short summary"));
        // Summary takes precedence over body
        assert!(!text.contains("Long body"));
    }
}
