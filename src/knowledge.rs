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

    // Issue #246: Trigger keywords for reactive memory injection
    #[serde(default)]
    pub triggers: Vec<String>, // Keywords/phrases that reactively inject this memory

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

    // Embedding chunks (Issue #346)
    #[serde(default)]
    pub chunk_count: i32,

    // Stele encoding format (Issue #122)
    #[serde(default = "default_format")]
    pub format: String, // markdown (default), json, stele:markdown, stele:ascii, stele:light, stele:full

    // Computed decay value (effective_resonance = resonance * decay factor).
    // None when decay hasn't been computed yet. Use this for resonance-sorted display;
    // raw `resonance` does not account for age.
    #[serde(default)]
    pub effective_resonance: Option<f64>,
}

/// Normalize a single trigger keyword/phrase for storage and matching (Issue #246).
///
/// Pipeline: Unicode NFC canonicalization, lowercase, trim, and collapse
/// internal whitespace runs to a single space. Returns `None` when the input is
/// empty after trimming, so callers can drop blank entries.
///
/// This is the single source of truth for trigger normalization, shared by the
/// store write path (PR1), the CLI flag parsing (PR2), and the trigger matcher
/// (PR3): all three MUST normalize identically so author-time and match-time
/// values line up.
///
/// NFC matters because precomposed "café" (U+00E9) and decomposed "café"
/// (e + U+0301 combining acute) are visually identical but byte-distinct. Author
/// time and match time can each arrive in either form (different keyboards,
/// editors, OSes), so we canonicalize BOTH sides to NFC here — the one shared
/// helper — guaranteeing they compare equal (Verdictia PR1 review, #246).
pub fn normalize_trigger(raw: &str) -> Option<String> {
    use unicode_normalization::UnicodeNormalization;
    // NFC first so combining sequences fold into precomposed code points before
    // we lowercase/compare. `to_lowercase` is locale-independent (Unicode default
    // case folding) and stable across the NFC-canonicalized input.
    let canonical: String = raw.nfc().collect();
    let collapsed = canonical.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.to_lowercase())
    }
}

/// Normalize a collection of triggers: apply `normalize_trigger` to each, drop
/// empties, and dedupe while preserving first-seen order (Issue #246).
pub fn normalize_triggers<I, S>(raw: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in raw {
        if let Some(norm) = normalize_trigger(item.as_ref())
            && seen.insert(norm.clone())
        {
            out.push(norm);
        }
    }
    out
}

/// Common LLM-regenerated Unicode punctuation glyphs that stand in for their
/// ASCII counterparts: smart quotes, en/em dash, and horizontal ellipsis
/// (fix-round review, minor finding: `is_ascii_punctuation` alone leaves
/// these untouched, so "don't" vs "don't" -- straight vs smart apostrophe --
/// fails to dedup even though it's the exact recase/repunctuate class this
/// gate targets). A false negative here only means a real duplicate slips
/// through uncaught, never a false-positive merge of distinct content, so
/// extending this list is one-directional safe.
const DEDUP_UNICODE_PUNCTUATION: [char; 7] = [
    '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2013}', '\u{2014}', '\u{2026}',
];

/// Normalize content for write-boundary DEDUPLICATION (W447). Lowercases,
/// strips ASCII punctuation plus the common Unicode punctuation glyphs LLM
/// regeneration substitutes for them (see [`DEDUP_UNICODE_PUNCTUATION`]), and
/// collapses whitespace, so a recased/repunctuated regenerated duplicate
/// ("The plan!" vs "the plan", or "don't" vs "don't") normalizes identically
/// to its original.
///
/// Distinct from [`KnowledgeEntry::normalize_content`], which stops at
/// case/whitespace folding and is depended on by thread-matching -- that
/// function is deliberately left unchanged. This is a separate, stricter
/// normalization used only by [`dedup_hash`].
pub fn normalize_for_dedup(content: &str) -> String {
    let no_punct: String = content
        .chars()
        .filter(|c| !c.is_ascii_punctuation() && !DEDUP_UNICODE_PUNCTUATION.contains(c))
        .collect();
    no_punct
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compute the write-boundary dedup hash for an entry's title+body (W447).
/// Two entries whose title+body normalize identically -- same text modulo
/// case, punctuation, and whitespace -- hash equal, so a
/// regenerated/recased near-duplicate is caught at the write boundary before
/// it lands as a second row.
///
/// Distinct from the legacy title-only `content_hash`
/// ([`KnowledgeEntry::compute_hash`], import-time change detection, computed
/// but never enforced) -- that field is untouched by this. `dedup_hash` keys
/// on title+body and is the write-boundary gate's identity key.
///
/// Hashes title and body as a length-prefixed PAIR, never a joined string
/// (fix-round review, finding 1): `normalize_for_dedup` collapses ALL
/// whitespace -- including any separator character we might pick, since
/// separators are themselves whitespace or get stripped as punctuation -- so
/// `dedup_hash("Ship it", "now.")` and `dedup_hash("Ship it now", "")`
/// previously collapsed to the identical normalized string "ship it now" and
/// hashed equal. That's exactly the variance an LLM regenerating the same
/// fact produces (folding a sentence into the title one turn, splitting it
/// title+body the next) -- a false-positive skip that silently drops the
/// second write. The length prefix on the normalized title makes the byte
/// stream fed to blake3 unambiguous: given the prefix, the title/body
/// boundary can always be recovered, so two different splits of the same
/// concatenated text can only hash equal if the title itself is
/// byte-identical too.
///
/// LANDMINE FOR THE NEXT EDITOR: this scheme is sound for exactly the two
/// fields hashed here -- the first field's length prefix pins the one
/// boundary that exists between two fields. If a third content field ever
/// gets folded into this hash (e.g. a summary or a tags string), it needs
/// its OWN length prefix too -- one prefix only disambiguates one boundary,
/// and an unprefixed third field reopens the exact ambiguity class this fix
/// closed (two different three-way splits of the same concatenated bytes
/// hashing equal).
///
/// `dedup_hash` is never persisted (recomputed from candidate rows on every
/// `ensure_group` call and from the incoming entry on every `check`), so
/// changing this algorithm has no migration concern -- there is no stored
/// value anywhere that this invalidates.
pub fn dedup_hash(title: &str, body: &str) -> String {
    let norm_title = normalize_for_dedup(title);
    let norm_body = normalize_for_dedup(body);
    let mut buf = Vec::with_capacity(norm_title.len() + norm_body.len() + 8);
    buf.extend_from_slice(&(norm_title.len() as u64).to_le_bytes());
    buf.extend_from_slice(norm_title.as_bytes());
    buf.extend_from_slice(norm_body.as_bytes());
    // `blake3_hex` has no `pub` but is visible here: Rust privacy is
    // module-scoped, and this free function lives in the same module
    // (knowledge.rs) as `impl KnowledgeEntry`.
    KnowledgeEntry::blake3_hex(&buf)
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
            parts.push(body.clone());
        }

        if !self.tags.is_empty() {
            parts.push(format!("Tags: {}", self.tags.join(", ")));
        }

        parts.join("\n\n")
    }

    /// Normalize content for comparison (thread matching, etc.)
    ///
    /// Trims, lowercases, and collapses internal whitespace runs to a single
    /// space. Does NOT strip punctuation -- "hello, world!" stays
    /// "hello, world!" (lowercased/collapsed). Thread-matching (helpers.rs)
    /// depends on this exact behavior, so this function is intentionally
    /// left unchanged; see `normalize_for_dedup` below for the punctuation-
    /// stripping variant used by write-boundary dedup (W447).
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
    fn test_normalize_trigger() {
        // Lowercase + trim + collapse internal whitespace
        assert_eq!(
            normalize_trigger("  Blood   Sugar "),
            Some("blood sugar".to_string())
        );
        assert_eq!(normalize_trigger("Brad"), Some("brad".to_string()));
        // Empty / whitespace-only -> None
        assert_eq!(normalize_trigger(""), None);
        assert_eq!(normalize_trigger("   "), None);
        assert_eq!(normalize_trigger("\t\n"), None);
    }

    #[test]
    fn test_normalize_trigger_nfc_canonicalization() {
        // Precomposed "café": ...caf + é (U+00E9).
        let precomposed = "caf\u{00e9}";
        // Decomposed "café": ...caf + e + combining acute (U+0301).
        let decomposed = "cafe\u{0301}";
        // Byte-distinct inputs...
        assert_ne!(precomposed, decomposed);
        // ...must normalize to the SAME canonical form (NFC). Without NFC in the
        // shared helper, a precomposed stored trigger would never fire on a
        // decomposed message token (or vice versa). #246 / Verdictia review.
        assert_eq!(
            normalize_trigger(precomposed),
            normalize_trigger(decomposed)
        );
        assert_eq!(normalize_trigger(decomposed), Some("café".to_string()));
    }

    #[test]
    fn test_normalize_triggers_dedupe_and_order() {
        let out = normalize_triggers(vec![
            "Brad",
            "  blood   sugar ",
            "BLOOD SUGAR", // dup after normalization
            "Glucose",
            "",     // dropped
            "   ",  // dropped
            "brad", // dup
        ]);
        assert_eq!(
            out,
            vec![
                "brad".to_string(),
                "blood sugar".to_string(),
                "glucose".to_string()
            ]
        );
    }

    #[test]
    fn test_normalize_for_dedup_strips_punctuation_case_and_whitespace() {
        assert_eq!(
            normalize_for_dedup("The EXTERNAL, numberless plan."),
            normalize_for_dedup("the external numberless plan")
        );
        assert_eq!(
            normalize_for_dedup("The EXTERNAL, numberless plan."),
            "the external numberless plan"
        );
    }

    #[test]
    fn test_normalize_for_dedup_strips_common_unicode_punctuation() {
        // Fix-round review, minor finding: smart quotes / em-dash / ellipsis
        // are the exact glyphs LLM regeneration swaps in for their ASCII
        // counterparts -- must fold to the same normalized form.
        assert_eq!(
            normalize_for_dedup("don't"),
            normalize_for_dedup("don\u{2019}t")
        );
        assert_eq!(
            normalize_for_dedup("\u{201C}quoted\u{201D}"),
            normalize_for_dedup("\"quoted\"")
        );
        assert_eq!(
            normalize_for_dedup("wait\u{2014}really"),
            normalize_for_dedup("wait-really")
        );
        assert_eq!(
            normalize_for_dedup("hold on\u{2026}"),
            normalize_for_dedup("hold on...")
        );
    }

    #[test]
    fn test_normalize_content_not_mutated_by_dedup_work() {
        // normalize_content must still lowercase/collapse WITHOUT stripping
        // punctuation -- thread-matching (helpers.rs) depends on the exact
        // pre-W447 behavior.
        assert_eq!(
            KnowledgeEntry::normalize_content("hello, world!"),
            "hello, world!"
        );
    }

    #[test]
    fn test_dedup_hash_recased_repunctuated_pair_equal() {
        // W447 evidence class: regenerated duplicates differ only by case,
        // punctuation, or whitespace -- dedup_hash must collapse them.
        let a = dedup_hash("The External Plan", "Ship it, and move on.");
        let b = dedup_hash("the external plan", "ship it and move on");
        assert_eq!(a, b);
    }

    #[test]
    fn test_dedup_hash_different_bodies_unequal() {
        let a = dedup_hash("The External Plan", "Ship it and move on.");
        let b = dedup_hash("The External Plan", "Ship it and hold off.");
        assert_ne!(a, b);
    }

    #[test]
    fn test_dedup_hash_does_not_collapse_title_body_boundary() {
        // Fix-round review, finding 1: pre-fix, both sides normalized to the
        // identical string "ship it now" (the `\n` joiner is whitespace, so
        // `normalize_for_dedup`'s split_whitespace/join collapses it exactly
        // like any other space) and hashed equal -- a false-positive skip
        // that silently drops a distinct entry. They must now differ.
        assert_ne!(dedup_hash("Ship it", "now."), dedup_hash("Ship it now", ""));

        // A second instance of the same class: the split point moves by one
        // word, and the total token stream still matches.
        assert_ne!(
            dedup_hash("Ship it now", "and move on"),
            dedup_hash("Ship it", "now and move on")
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
            triggers: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            chunk_count: 0,
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
            triggers: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            chunk_count: 0,
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
