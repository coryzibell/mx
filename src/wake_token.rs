use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::knowledge::KnowledgeEntry;
use crate::store::WakeCascade;

type HmacSha256 = Hmac<Sha256>;

/// Session token for chained wake ritual
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeSessionToken {
    pub session_id: String,
    pub bloom_ids: Vec<String>,
    pub current_index: usize,
    pub attempts_on_current: u8,
    pub remembered_count: u32,
    pub needed_help_count: u32,
    pub skipped_count: u32,
    pub created_at: i64,
}

impl WakeSessionToken {
    /// Create new session from cascade
    pub fn new(cascade: &WakeCascade) -> Self {
        let mut bloom_ids = Vec::new();
        bloom_ids.extend(cascade.core.iter().map(|e| e.id.clone()));
        bloom_ids.extend(cascade.recent.iter().map(|e| e.id.clone()));
        bloom_ids.extend(cascade.bridges.iter().map(|e| e.id.clone()));

        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            bloom_ids,
            current_index: 0,
            attempts_on_current: 0,
            remembered_count: 0,
            needed_help_count: 0,
            skipped_count: 0,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    /// Get current bloom ID
    pub fn current_bloom_id(&self) -> Option<&str> {
        self.bloom_ids.get(self.current_index).map(|s| s.as_str())
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

    /// Sign the token
    pub fn sign(&self) -> Result<String> {
        let secret = get_wake_secret();
        let payload = serde_json::to_string(self)?;
        let payload_b64 = general_purpose::STANDARD.encode(payload.as_bytes());

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
        mac.update(payload.as_bytes());
        let signature = mac.finalize().into_bytes();
        let signature_b64 = general_purpose::STANDARD.encode(signature);

        Ok(format!("{}.{}", payload_b64, signature_b64))
    }

    /// Verify and decode a signed token
    pub fn verify(token_str: &str) -> Result<Self> {
        let secret = get_wake_secret();
        let parts: Vec<&str> = token_str.split('.').collect();

        if parts.len() != 2 {
            bail!("Invalid token format");
        }

        let payload_b64 = parts[0];
        let signature_b64 = parts[1];

        let payload = general_purpose::STANDARD
            .decode(payload_b64)
            .map_err(|_| anyhow::anyhow!("Invalid base64 in payload"))?;
        let provided_sig = general_purpose::STANDARD
            .decode(signature_b64)
            .map_err(|_| anyhow::anyhow!("Invalid base64 in signature"))?;

        // Verify signature
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {}", e))?;
        mac.update(&payload);
        mac.verify_slice(&provided_sig)
            .map_err(|_| anyhow::anyhow!("Invalid token signature"))?;

        // Deserialize
        let token: WakeSessionToken = serde_json::from_slice(&payload)?;
        Ok(token)
    }
}

/// Get wake secret from environment or use default
fn get_wake_secret() -> String {
    std::env::var("MX_WAKE_SECRET").unwrap_or_else(|_| "tsunderground-wake-ritual-v1".to_string())
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
    pub max_attempts: Option<u8>,
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
    pub has_wake_phrase: bool,
}

#[derive(Debug, Serialize)]
pub struct BloomFull {
    pub id: String,
    pub title: String,
    pub content: String,
    pub resonance: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resonance_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wake_phrase: Option<String>,
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
        Self {
            id: entry.id.clone(),
            title: entry.title.clone(),
            resonance: entry.resonance,
            resonance_type: entry.resonance_type.clone(),
            has_wake_phrase: entry.wake_phrase.is_some(),
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

        Self {
            id: entry.id.clone(),
            title: entry.title.clone(),
            content,
            resonance: entry.resonance,
            resonance_type: entry.resonance_type.clone(),
            wake_phrase: entry.wake_phrase.clone(),
        }
    }
}
