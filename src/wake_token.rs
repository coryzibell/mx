use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::knowledge::KnowledgeEntry;
use crate::store::WakeCascade;

type HmacSha256 = Hmac<Sha256>;

/// Create a signed wake ritual token: `{session_id}.{current_index}.{truncated_hmac[..16]}`
///
/// State lives server-side in SurrealDB. The token is a compact signed reference
/// that changes each step, providing integrity (HMAC), anti-replay (step must match),
/// and progression visibility.
pub fn create_token(session_id: &str, current_index: usize) -> String {
    let payload = format!("{}.{}", session_id, current_index);

    let key = format!("wake-{}-ritual", session_id);
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let signature = BASE64.encode(mac.finalize().into_bytes());

    format!("{}.{}", payload, &signature[..16])
}

/// Verify a wake ritual token and extract (session_id, current_index).
///
/// Token format: `{session_id}.{current_index}.{truncated_hmac[..16]}`
/// Since session_id is a UUID (contains hyphens but no dots), we split on '.'
/// to get exactly 3 parts.
pub fn verify_token(token: &str) -> Result<(String, usize), String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid token format".to_string());
    }

    let session_id = parts[0];
    let current_index: usize = parts[1]
        .parse()
        .map_err(|_| "Invalid current index in token".to_string())?;
    let provided_sig = parts[2];

    // Verify HMAC signature
    let payload = format!("{}.{}", session_id, current_index);
    let key = format!("wake-{}-ritual", session_id);
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    let expected_sig = BASE64.encode(mac.finalize().into_bytes());

    if &expected_sig[..16] != provided_sig {
        return Err("Invalid token signature".to_string());
    }

    Ok((session_id.to_string(), current_index))
}

/// Server-side wake ritual session state.
///
/// Persisted in SurrealDB's `wake_session` table. The CLI passes a compact
/// signed token (`{session_id}.{step}.{hmac}`) between calls. State is
/// server-side; the token is just a signed reference with anti-replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeSession {
    pub session_id: String,
    pub bloom_ids: Vec<String>,
    pub current_index: usize,
    pub attempts_on_current: u8,
    pub remembered_count: u32,
    pub needed_help_count: u32,
    pub skipped_count: u32,
    pub created_at: i64,
    /// Selected phrase indices for each bloom (None if bloom has no phrases)
    pub selected_phrase_indices: Vec<Option<usize>>,
}

impl WakeSession {
    /// Create new session from cascade
    pub fn new(cascade: &WakeCascade) -> Self {
        use rand::Rng;

        let mut bloom_ids = Vec::new();
        let mut selected_phrase_indices = Vec::new();

        // Collect blooms and select phrase indices
        for entry in cascade
            .core
            .iter()
            .chain(cascade.recent.iter())
            .chain(cascade.bridges.iter())
        {
            bloom_ids.push(entry.id.clone());

            // Select random phrase index if bloom has phrases
            let phrase_idx = if !entry.wake_phrases.is_empty() {
                Some(rand::rng().random_range(0..entry.wake_phrases.len()))
            } else if entry.wake_phrase.is_some() {
                Some(0) // Single wake_phrase maps to index 0
            } else {
                None // No phrases
            };
            selected_phrase_indices.push(phrase_idx);
        }

        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            bloom_ids,
            current_index: 0,
            attempts_on_current: 0,
            remembered_count: 0,
            needed_help_count: 0,
            skipped_count: 0,
            created_at: chrono::Utc::now().timestamp(),
            selected_phrase_indices,
        }
    }

    /// Get current bloom ID
    pub fn current_bloom_id(&self) -> Option<&str> {
        self.bloom_ids.get(self.current_index).map(|s| s.as_str())
    }

    /// Get selected phrase index for current bloom
    pub fn current_phrase_index(&self) -> Option<usize> {
        self.selected_phrase_indices
            .get(self.current_index)
            .and_then(|&idx| idx)
    }

    /// Total blooms in session
    pub fn total(&self) -> usize {
        self.bloom_ids.len()
    }

    /// Current position (1-indexed for display)
    pub fn current_position(&self) -> usize {
        self.current_index + 1
    }

    /// Check if ritual is complete
    pub fn is_complete(&self) -> bool {
        self.current_index >= self.bloom_ids.len()
    }

    /// Advance to next bloom (remembered)
    pub fn advance_remembered(&mut self) {
        self.remembered_count += 1;
        self.current_index += 1;
        self.attempts_on_current = 0;
    }

    /// Advance to next bloom (needed help)
    pub fn advance_helped(&mut self) {
        self.needed_help_count += 1;
        self.current_index += 1;
        self.attempts_on_current = 0;
    }

    /// Advance to next bloom (skipped)
    pub fn advance_skipped(&mut self) {
        self.skipped_count += 1;
        self.current_index += 1;
        self.attempts_on_current = 0;
    }

    /// Increment attempt counter
    pub fn increment_attempt(&mut self) {
        self.attempts_on_current += 1;
    }
}

/// JSON output structures

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
    pub matched_phrase: Option<String>, // Which phrase was matched/selected
}

#[derive(Debug, Serialize)]
pub struct Progress {
    pub current: usize,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remembered: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needed_help: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub total: usize,
    pub remembered: u32,
    pub needed_help: u32,
    pub skipped: u32,
}

/// Convert KnowledgeEntry to BloomPrompt
impl From<&KnowledgeEntry> for BloomPrompt {
    fn from(entry: &KnowledgeEntry) -> Self {
        let phrase_count = if !entry.wake_phrases.is_empty() {
            entry.wake_phrases.len()
        } else if entry.wake_phrase.is_some() {
            1
        } else {
            0
        };

        Self {
            id: entry.id.clone(),
            title: entry.title.clone(),
            resonance: entry.resonance,
            resonance_type: entry.resonance_type.clone(),
            wake_phrase_count: phrase_count,
        }
    }
}

/// Convert KnowledgeEntry to BloomFull
impl From<&KnowledgeEntry> for BloomFull {
    fn from(entry: &KnowledgeEntry) -> Self {
        let content = entry
            .body
            .clone()
            .or_else(|| entry.summary.clone())
            .unwrap_or_else(|| "(no content)".to_string());

        // Collect all phrases (wake_phrases array takes priority)
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
            matched_phrase: None, // Not set by default, populated during ritual
        }
    }
}
