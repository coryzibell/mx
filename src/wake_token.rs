use serde::{Deserialize, Serialize};

use crate::knowledge::KnowledgeEntry;
use crate::store::WakeCascade;

/// Server-side wake ritual session state.
///
/// Previously stored in a signed client token (JWT-like). Now persisted in
/// SurrealDB's `wake_session` table. The CLI passes only the UUID session_id
/// between calls instead of the full signed blob.
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
