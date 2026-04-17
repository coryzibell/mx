use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

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

/// Per-bloom chunking metadata stored on the session. Pure-runtime-projection:
/// the actual chunk plan is recomputed from current content on every
/// `respond`/`skip` call; this metadata only tracks how many authored phrases
/// the bloom had (which shapes the authored-vs-derived decision) and whether
/// the bloom has any phrases at all (P==0 case stays skip-type across all
/// chunks, per the conservative P==0 decision).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BloomChunkMeta {
    /// Number of authored wake phrases on this bloom at session start.
    /// Chunks with `chunk_idx < authored_phrase_count` use the authored phrase
    /// at that index; chunks beyond it derive a phrase from their own content.
    ///
    /// Widened u8→u16 on rebase onto merged #212 so it can compare directly
    /// against `chunk_idx: u16` without cross-width casts at every site.
    /// Realistic values stay ≤10 in practice; the wider type is uniformity.
    pub authored_phrase_count: u16,
    /// If true, this bloom has zero authored phrases — every chunk emits as
    /// skip-type (conservative-by-default P==0 decision). Never auto-derived.
    pub is_phraseless: bool,
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
    /// chunk advance (remembered / helped / skipped). Survives bloom
    /// re-chunking mid-ritual.
    pub step: u32,
    pub attempts_on_current: u8,
    pub remembered_count: u32,
    pub needed_help_count: u32,
    pub skipped_count: u32,
    pub created_at: i64,
    /// Per-bloom metadata (1:1 with `bloom_ids`). Replaces the old
    /// `selected_phrase_indices` representation — we no longer pre-select a
    /// random phrase, because chunks walk through authored phrases in index
    /// order and auto-derive beyond that.
    pub bloom_chunk_meta: Vec<BloomChunkMeta>,
}

impl WakeSession {
    /// Create new session from cascade. Computes per-bloom phrase counts up
    /// front so the `phrase_for_chunk` selector has deterministic metadata
    /// to consult, but does NOT pre-compute chunk plans — those are
    /// re-derived from fresh content on every `respond`/`skip` call.
    pub fn new(cascade: &WakeCascade) -> Self {
        let mut bloom_ids = Vec::new();
        let mut bloom_chunk_meta = Vec::new();

        for entry in cascade
            .core
            .iter()
            .chain(cascade.recent.iter())
            .chain(cascade.bridges.iter())
        {
            bloom_ids.push(entry.id.clone());

            let authored_phrase_count = authored_phrase_count(entry);
            bloom_chunk_meta.push(BloomChunkMeta {
                authored_phrase_count,
                is_phraseless: authored_phrase_count == 0,
            });
        }

        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            bloom_ids,
            current_index: 0,
            current_chunk_index: 0,
            step: 0,
            attempts_on_current: 0,
            remembered_count: 0,
            needed_help_count: 0,
            skipped_count: 0,
            created_at: chrono::Utc::now().timestamp(),
            bloom_chunk_meta,
        }
    }

    /// Get current bloom ID
    pub fn current_bloom_id(&self) -> Option<&str> {
        self.bloom_ids.get(self.current_index).map(|s| s.as_str())
    }

    /// Per-bloom chunk metadata for the current bloom, if any.
    pub fn current_meta(&self) -> Option<&BloomChunkMeta> {
        self.bloom_chunk_meta.get(self.current_index)
    }

    /// Total blooms in session
    pub fn total_blooms(&self) -> usize {
        self.bloom_ids.len()
    }

    /// Current bloom position (1-indexed for display)
    pub fn current_bloom_position(&self) -> usize {
        self.current_index + 1
    }

    /// Check if ritual is complete
    pub fn is_complete(&self) -> bool {
        self.current_index >= self.bloom_ids.len()
    }

    /// Advance past the current chunk. If there are more chunks in this
    /// bloom (per `bloom_total_chunks`), tick `current_chunk_index`; otherwise
    /// advance to the next bloom and reset chunk cursor. Always ticks `step`.
    ///
    /// Assertion-heavy by design (Risk 4): off-by-one bugs here will serve
    /// wrong content or stick the ritual.
    pub fn advance_remembered(&mut self, bloom_total_chunks: u16) {
        debug_assert!(!self.is_complete(), "advance called on completed session");
        debug_assert!(
            (self.current_chunk_index as usize) < bloom_total_chunks.max(1) as usize,
            "current_chunk_index {} >= bloom_total_chunks {}",
            self.current_chunk_index,
            bloom_total_chunks
        );
        self.remembered_count += 1;
        self.step = self.step.saturating_add(1);
        self.advance_chunk_or_bloom(bloom_total_chunks);
    }

    /// Advance past the current chunk (needed help path).
    pub fn advance_helped(&mut self, bloom_total_chunks: u16) {
        debug_assert!(!self.is_complete());
        self.needed_help_count += 1;
        self.step = self.step.saturating_add(1);
        self.advance_chunk_or_bloom(bloom_total_chunks);
    }

    /// Advance past the current chunk (skipped path).
    pub fn advance_skipped(&mut self, bloom_total_chunks: u16) {
        debug_assert!(!self.is_complete());
        self.skipped_count += 1;
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
            self.attempts_on_current = 0;
        } else {
            // Move to the next bloom; reset chunk cursor.
            self.current_index += 1;
            self.current_chunk_index = 0;
            self.attempts_on_current = 0;
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
            self.attempts_on_current = 0;
            true
        } else {
            false
        }
    }

    /// Increment attempt counter
    pub fn increment_attempt(&mut self) {
        self.attempts_on_current += 1;
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

/// The authored phrase at the given index, if it exists. Consolidates the
/// `wake_phrases[idx]` vs legacy `wake_phrase` lookup.
pub fn authored_phrase_at(entry: &KnowledgeEntry, idx: usize) -> Option<String> {
    if !entry.wake_phrases.is_empty() {
        entry.wake_phrases.get(idx).cloned()
    } else if idx == 0 {
        entry.wake_phrase.clone()
    } else {
        None
    }
}

// ============================================================================
// JSON output structures — strictly additive vs the previous contract
// ============================================================================

#[derive(Debug, Serialize)]
pub struct WakeBeginResponse {
    pub status: String,
    pub session: String,
    pub prompt: BloomPrompt,
    pub progress: Progress,
}

#[derive(Debug, Serialize)]
pub struct WakeRespondResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bloom: Option<BloomFull>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<BloomPrompt>,
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<BloomPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    /// Set to `Some(true)` when the consumer's response failed to match a
    /// phrase that was auto-derived from chunk content (as opposed to an
    /// authored phrase from the bloom owner).
    ///
    /// **Honest semantics:** this field fires on ANY derived-phrase
    /// mismatch — it does NOT guarantee the bloom content actually changed
    /// during the ritual. The original design (§10 Risk 9) proposed a
    /// timestamp-compare (`bloom.updated_at > session.created_at`) to
    /// distinguish "content genuinely shifted mid-ritual" from "user typed
    /// the wrong thing"; `KnowledgeEntry.updated_at` is `Option<String>`
    /// (RFC3339 requiring parsing) so that tighter check is deferred.
    ///
    /// Renamed from `content_changed_during_ritual` after Diffi's mx#213
    /// review called out the name as overpromising. Consumers should treat
    /// this as an advisory "you guessed a sampled phrase and it didn't
    /// match — if you edited the bloom mid-ritual, consider a `--begin`
    /// restart; otherwise just try again." Not a content-change detector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_phrase_mismatch: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct WakeSkipResponse {
    pub status: String,
    pub bloom: BloomFull,
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<BloomPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
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
    pub title: String,
    pub resonance: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resonance_type: Option<String>,
    pub wake_phrase_count: usize,
    /// Present only for chunked blooms. `{index: 1-based, total}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ChunkRef>,
    /// `"authored"` | `"derived"` — advisory field so consumers can surface
    /// that a phrase was sampled from chunk content rather than authored by
    /// the bloom owner. Absent for non-chunked or phraseless blooms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phrase_source: Option<String>,
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
    pub title: String,
    pub content: String,
    pub resonance: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resonance_type: Option<String>,
    pub all_phrases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_phrase: Option<String>,
    /// Chunk metadata for the chunk being returned, if the bloom was chunked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ChunkRef>,
    /// `"authored"` | `"derived"` — which phrase type unlocked this chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phrase_source: Option<String>,
    /// `true` if the session's chunk cursor was clamped forward because the
    /// bloom shrank past it mid-ritual. Observability for §2.2.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_truncated: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Progress {
    /// 1-indexed count of chunks walked into. Counts chunks, not blooms
    /// (the unit of progression in the new flow).
    pub current: usize,
    /// Total chunks across the whole cascade (eager at begin; may drift by
    /// ≤10% if mid-ritual edits change bloom sizes — see §7.1).
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remembered: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needed_help: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u32>,
    /// 1-indexed bloom counter. Consumers that prefer the old "X of N blooms"
    /// UX can render this instead of `current`/`total`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bloom_current: Option<usize>,
    /// Total blooms in the cascade. Stable across the ritual.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bloom_total: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub total: usize,
    pub remembered: u32,
    pub needed_help: u32,
    pub skipped: u32,
    /// Per-bloom roll-up (chunks remembered/helped/skipped grouped by bloom).
    /// Populated in PR 3; kept here as an optional field for PR 2 so the
    /// payload shape doesn't change again between PRs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blooms_complete: Option<Vec<BloomRollup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_remembered: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_skipped: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_needed_help: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct BloomRollup {
    pub id: String,
    pub title: String,
    pub chunks: String,
}

/// Convert KnowledgeEntry to BloomPrompt (non-chunked form — PR 2 wires a
/// chunk-aware builder in the ritual module).
impl From<&KnowledgeEntry> for BloomPrompt {
    fn from(entry: &KnowledgeEntry) -> Self {
        let phrase_count = authored_phrase_count(entry) as usize;

        Self {
            id: entry.id.clone(),
            title: entry.title.clone(),
            resonance: entry.resonance,
            resonance_type: entry.resonance_type.clone(),
            wake_phrase_count: phrase_count,
            chunk: None,
            phrase_source: None,
        }
    }
}

/// Convert KnowledgeEntry to BloomFull (non-chunked form — PR 2 wires a
/// chunk-aware builder in the ritual module).
impl From<&KnowledgeEntry> for BloomFull {
    fn from(entry: &KnowledgeEntry) -> Self {
        let content = entry
            .body
            .clone()
            .or_else(|| entry.summary.clone())
            .unwrap_or_else(|| "(no content)".to_string());

        let all_phrases = if !entry.wake_phrases.is_empty() {
            entry.wake_phrases.clone()
        } else if let Some(ref phrase) = entry.wake_phrase {
            vec![phrase.clone()]
        } else {
            vec![]
        };

        Self {
            title: entry.title.clone(),
            content,
            resonance: entry.resonance,
            resonance_type: entry.resonance_type.clone(),
            all_phrases,
            matched_phrase: None,
            chunk: None,
            phrase_source: None,
            chunk_truncated: None,
        }
    }
}
