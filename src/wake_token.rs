use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;

use crate::knowledge::KnowledgeEntry;
use crate::store::WakeCascade;

type HmacSha256 = Hmac<Sha256>;

/// Create a signed wake ritual token: `{session_id}.{step}.{truncated_hmac[..16]}`.
///
/// `step` is a monotonic counter of chunks walked (not bloom index). The wire
/// format is unchanged from previous versions — the middle segment still parses
/// as an integer — but the semantics shift to "cumulative chunks walked" so
/// that mid-ritual bloom edits (which can change chunk counts) don't invalidate
/// previously-issued tokens. See mx#211 §2.3 / §6.
pub fn create_token(session_id: &str, step: u32) -> String {
    let payload = format!("{}.{}", session_id, step);

    let key = format!("wake-{}-ritual", session_id);
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let signature = BASE64.encode(mac.finalize().into_bytes());

    format!("{}.{}", payload, &signature[..16])
}

/// Verify a wake ritual token and extract (session_id, step).
///
/// Token format: `{session_id}.{step}.{truncated_hmac[..16]}`. `step` is the
/// monotonic chunk counter; see `create_token`.
pub fn verify_token(token: &str) -> Result<(String, u32), String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid token format".to_string());
    }

    let session_id = parts[0];
    let step: u32 = parts[1]
        .parse()
        .map_err(|_| "Invalid step in token".to_string())?;
    let provided_sig = parts[2];

    let payload = format!("{}.{}", session_id, step);
    let key = format!("wake-{}-ritual", session_id);
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let expected_sig = BASE64.encode(mac.finalize().into_bytes());

    if &expected_sig[..16] != provided_sig {
        return Err("Invalid token signature".to_string());
    }

    Ok((session_id.to_string(), step))
}

/// Where the phrase a chunk was matched against came from — authored by the
/// bloom owner, derived from the chunk's own content, or auto-generated for a
/// bloom with no authored phrases at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseSource {
    Authored,
    Derived,
    Auto,
}

impl PhraseSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PhraseSource::Authored => "authored",
            PhraseSource::Derived => "derived",
            PhraseSource::Auto => "auto",
        }
    }
}

/// Bucket counts split by phrase source. A string match against an `auto`
/// phrase that fell through to the title itself means nothing, so the split
/// keeps that visible instead of folding it into one number.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceCounts {
    #[serde(default)]
    pub authored: u32,
    #[serde(default)]
    pub derived: u32,
    #[serde(default)]
    pub auto: u32,
}

impl SourceCounts {
    fn bump(&mut self, source: PhraseSource) {
        match source {
            PhraseSource::Authored => self.authored += 1,
            PhraseSource::Derived => self.derived += 1,
            PhraseSource::Auto => self.auto += 1,
        }
    }

    fn total(&self) -> u32 {
        self.authored + self.derived + self.auto
    }

    fn add(&mut self, other: &SourceCounts) {
        self.authored += other.authored;
        self.derived += other.derived;
        self.auto += other.auto;
    }
}

/// Per-bloom outcome counters (1:1 with `bloom_ids`). The chunk plan itself is
/// recomputed from current content on every respond call; only the outcomes
/// accumulate here, and the final summary is their sum.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct BloomChunkMeta {
    #[serde(default)]
    pub unhinted: SourceCounts,
    #[serde(default)]
    pub revealed: SourceCounts,
}

/// Server-side wake ritual session state.
///
/// Persisted in SurrealDB's `wake_session` table. The CLI passes a compact
/// signed token (`{session_id}.{step}.{hmac}`) between calls. State is
/// server-side; the token is just a signed reference with anti-replay.
///
/// ## Cursor invariants (Risk 4 in the design)
///
/// Two cursors compose a single position:
///
/// - `current_index` — which bloom we're on in `bloom_ids`. `0..=bloom_ids.len()`.
/// - `current_chunk_index` — which chunk within the current bloom. Always
///   advances to 0 when `current_index` advances. `0..=chunk_plan.total` for
///   the current bloom's plan.
///
/// `step` is the monotonic count of chunks walked. It is independent of
/// `current_index` / `current_chunk_index` — the latter can drift if bloom
/// content changes mid-ritual and re-chunks, but `step` always increments by
/// exactly 1 per chunk advance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeSession {
    pub session_id: String,
    /// The agent that began the ritual. Carried onto every guess row, and the
    /// key every guess-log query filters on.
    pub agent: String,
    /// Wake number supplied by the caller via `--wake`. mx has no counter of
    /// its own, so this is `None` for callers that do not pass one.
    pub wake: Option<i64>,
    /// Model identifier supplied by the caller via `--model`. mx has no way to
    /// discover it.
    pub model_id: Option<String>,
    pub bloom_ids: Vec<String>,
    /// Which bloom we're on. 0-indexed; equals `bloom_ids.len()` when the
    /// ritual is complete.
    pub current_index: usize,
    /// Which chunk within the current bloom we're on. Resets to 0 when
    /// `current_index` advances. For non-chunked blooms this stays 0.
    ///
    /// Widened u8→u16 on rebase onto merged #212 — aligns with
    /// `ChunkPlan.total: u16` so large-bloom-with-low-threshold rituals
    /// (chunks > 255) address chunks correctly. Typical values remain 0-3.
    pub current_chunk_index: u16,
    /// Monotonic step counter used for token anti-replay. Ticks by 1 on every
    /// chunk advance. Survives bloom re-chunking mid-ritual.
    pub step: u32,
    pub unhinted_count: u32,
    pub revealed_count: u32,
    pub created_at: i64,
    /// Per-bloom outcome counters (1:1 with `bloom_ids`).
    pub bloom_chunk_meta: Vec<BloomChunkMeta>,
}

impl WakeSession {
    /// Create a new session from a cascade. Chunk plans are NOT pre-computed —
    /// they are re-derived from fresh content on every respond call.
    pub fn new(
        cascade: &WakeCascade,
        agent: String,
        wake: Option<i64>,
        model_id: Option<String>,
    ) -> Self {
        let bloom_ids: Vec<String> = cascade
            .core
            .iter()
            .chain(cascade.recent.iter())
            .chain(cascade.bridges.iter())
            .map(|entry| entry.id.clone())
            .collect();
        let bloom_chunk_meta = vec![BloomChunkMeta::default(); bloom_ids.len()];

        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            agent,
            wake,
            model_id,
            bloom_ids,
            current_index: 0,
            current_chunk_index: 0,
            step: 0,
            unhinted_count: 0,
            revealed_count: 0,
            created_at: chrono::Utc::now().timestamp(),
            bloom_chunk_meta,
        }
    }

    /// Get current bloom ID
    pub fn current_bloom_id(&self) -> Option<&str> {
        self.bloom_ids.get(self.current_index).map(|s| s.as_str())
    }

    /// Total blooms in session
    pub fn total_blooms(&self) -> usize {
        self.bloom_ids.len()
    }

    /// Sum of the per-bloom bucket counters across the whole ritual.
    pub fn bucket_totals(&self) -> Buckets {
        let mut buckets = Buckets::default();
        for meta in &self.bloom_chunk_meta {
            buckets.unhinted.add(&meta.unhinted);
            buckets.revealed.add(&meta.revealed);
        }
        buckets
    }

    /// Current bloom position (1-indexed for display)
    pub fn current_bloom_position(&self) -> usize {
        self.current_index + 1
    }

    /// Check if ritual is complete
    pub fn is_complete(&self) -> bool {
        self.current_index >= self.bloom_ids.len()
    }

    /// Record the outcome for the current chunk and advance past it. If there
    /// are more chunks in this bloom (per `bloom_total_chunks`), tick
    /// `current_chunk_index`; otherwise advance to the next bloom and reset
    /// the chunk cursor. Always ticks `step`.
    ///
    /// Assertion-heavy by design (Risk 4): off-by-one bugs here will serve
    /// wrong content or stick the ritual.
    pub fn advance(
        &mut self,
        bloom_total_chunks: u16,
        bucket: crate::wake_guess::Bucket,
        source: PhraseSource,
    ) {
        debug_assert!(!self.is_complete(), "advance called on completed session");
        debug_assert!(
            (self.current_chunk_index as usize) < bloom_total_chunks.max(1) as usize,
            "current_chunk_index {} >= bloom_total_chunks {}",
            self.current_chunk_index,
            bloom_total_chunks
        );
        match bucket {
            crate::wake_guess::Bucket::Unhinted => self.unhinted_count += 1,
            crate::wake_guess::Bucket::Revealed => self.revealed_count += 1,
        }
        if let Some(meta) = self.bloom_chunk_meta.get_mut(self.current_index) {
            match bucket {
                crate::wake_guess::Bucket::Unhinted => meta.unhinted.bump(source),
                crate::wake_guess::Bucket::Revealed => meta.revealed.bump(source),
            }
        }
        self.step = self.step.saturating_add(1);
        self.advance_chunk_or_bloom(bloom_total_chunks);
    }

    /// Core cursor advance. Pure function of the two cursors + the chunk
    /// total. Called by the three `advance_*` wrappers above.
    fn advance_chunk_or_bloom(&mut self, bloom_total_chunks: u16) {
        let next_chunk = self.current_chunk_index.saturating_add(1);
        if (next_chunk as usize) < bloom_total_chunks.max(1) as usize {
            // More chunks in this bloom.
            self.current_chunk_index = next_chunk;
        } else {
            // Move to the next bloom; reset chunk cursor.
            self.current_index += 1;
            self.current_chunk_index = 0;
        }
        debug_assert!(
            self.current_index <= self.bloom_ids.len(),
            "current_index {} overshot bloom_ids.len() {}",
            self.current_index,
            self.bloom_ids.len()
        );
    }

    /// Handle the "content shrank mid-ritual past the cursor" case (§2.2): if
    /// the recomputed chunk plan has fewer chunks than `current_chunk_index`,
    /// we clamp and advance to the next bloom. Flagged as `chunk_truncated`
    /// in the response for observability.
    ///
    /// Returns `true` if clamping occurred (caller should set the
    /// `chunk_truncated` response field).
    pub fn clamp_if_chunks_shrank(&mut self, bloom_total_chunks: u16) -> bool {
        let total = bloom_total_chunks.max(1) as usize;
        if (self.current_chunk_index as usize) >= total {
            self.current_index += 1;
            self.current_chunk_index = 0;
            true
        } else {
            false
        }
    }
}

/// Count of authored wake phrases on an entry (wake_phrases takes priority
/// over the legacy single `wake_phrase`).
pub fn authored_phrase_count(entry: &KnowledgeEntry) -> u16 {
    if !entry.wake_phrases.is_empty() {
        u16::try_from(entry.wake_phrases.len()).unwrap_or(u16::MAX)
    } else if entry.wake_phrase.is_some() {
        1
    } else {
        0
    }
}

/// Every authored phrase on an entry, in author order. `wake_phrases` takes
/// priority over the legacy single `wake_phrase`.
pub fn authored_phrases(entry: &KnowledgeEntry) -> Vec<String> {
    if !entry.wake_phrases.is_empty() {
        entry.wake_phrases.clone()
    } else if let Some(ref phrase) = entry.wake_phrase {
        vec![phrase.clone()]
    } else {
        Vec::new()
    }
}

// ============================================================================
// JSON output structures
// ============================================================================

#[derive(Debug, Serialize)]
pub struct WakeBeginResponse {
    pub status: String,
    pub session: String,
    pub prompt: BloomPrompt,
    pub progress: Progress,
    /// Per-tag counts of entries that met the cascade's other criteria and
    /// were dropped because they carry an excluded tag. Empty when nothing
    /// was dropped.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub excluded: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct WakeRespondResponse {
    /// `shown` after a guess was judged, or `chunk_truncated` when the bloom
    /// shrank past the session's chunk cursor and no guess was judged.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guess: Option<String>,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_info: Option<MatchInfo>,
    pub bloom: BloomFull,
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<BloomPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
}

/// A mechanical string-match fact. `kind` is `exact`, `close` or `none`.
#[derive(Debug, Serialize)]
pub struct MatchInfo {
    pub kind: String,
    /// Index into `bloom.phrases` of the phrase that matched. Null on `none`.
    pub phrase_index: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct WakeErrorResponse {
    pub status: String,
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BloomPrompt {
    pub id: String,
    /// The title as shown, including any `(Part N/M)` suffix.
    pub title: String,
    pub phrase_source: String,
    /// Present only for chunked blooms. `{index: 1-based, total}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ChunkRef>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChunkRef {
    pub index: u16,
    pub total: u16,
    /// Present and `true` when the chunk exceeds the chunking threshold
    /// (typically an un-splittable code block). Documented limitation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oversized: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct BloomFull {
    pub id: String,
    pub title: String,
    /// The phrases the guess was matched against. For `derived` and `auto`
    /// sources this holds the single generated phrase.
    pub phrases: Vec<String>,
    pub phrase_source: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ChunkRef>,
}

#[derive(Debug, Serialize)]
pub struct Progress {
    /// 1-indexed count of chunks walked into.
    pub current: usize,
    /// Total chunks across the whole cascade.
    pub total: usize,
    /// 1-indexed bloom counter, and the total blooms in the cascade.
    pub bloom_current: usize,
    pub bloom_total: usize,
    /// Running bucket totals. Absent on the begin response, where nothing has
    /// been guessed yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buckets: Option<BucketTotals>,
}

/// Running totals, unsplit. The per-source split is only in the final summary.
#[derive(Debug, Serialize)]
pub struct BucketTotals {
    pub unhinted: u32,
    pub revealed: u32,
}

/// Everything the model sees at the end of a ritual: what the tool did, split
/// by phrase source. No total, no ratio, no similarity value.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub chunks: usize,
    pub blooms: usize,
    pub buckets: Buckets,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Buckets {
    pub unhinted: SourceCounts,
    pub revealed: SourceCounts,
}

impl Buckets {
    pub fn totals(&self) -> BucketTotals {
        BucketTotals {
            unhinted: self.unhinted.total(),
            revealed: self.revealed.total(),
        }
    }
}
