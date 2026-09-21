//! The wake ritual's first-guess log.
//!
//! Every `--respond` call that reaches matching writes exactly one row to the
//! `wake_guess` table before the session advances. Under the one-guess ladder
//! there is no second attempt, so every row is a first guess. The row is the
//! data the ritual exists to collect: a failed write fails the respond call.
//!
//! The similarity columns (`embedding`, `sim_*`, `scored_at`) are defined in
//! the schema but are never written here — they are filled by a later scoring
//! pass, and a row with a null `scored_at` is pending.

use sha2::{Digest, Sha256};

/// Which bucket a guess landed in.
///
/// The names claim only what the tool did, not what the responder knew:
/// blooms shown earlier in a ritual leak into later guesses and the tool
/// cannot see that. `position` on the row separates early from late.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// The guess string-matched a phrase, and the tool had given no hint.
    Unhinted,
    /// The guess did not string-match, and the bloom was shown.
    Revealed,
}

impl Bucket {
    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Unhinted => "unhinted",
            Bucket::Revealed => "revealed",
        }
    }
}

/// A mechanical string-match fact about one guess. Not a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Close,
    None,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::Exact => "exact",
            MatchKind::Close => "close",
            MatchKind::None => "none",
        }
    }
}

/// One `wake_guess` row, as written at respond time.
#[derive(Debug, Clone)]
pub struct WakeGuessRow {
    pub agent: String,
    /// Wake number from `--wake`. Absent for callers that do not count wakes.
    pub wake: Option<i64>,
    pub session_id: String,
    pub bloom_id: String,
    /// 0-based chunk within the bloom, and the bloom's chunk count.
    pub chunk_index: u16,
    pub chunk_total: u16,
    /// 0-based chunk step within the ritual (the session's `step` at guess
    /// time). Advances on every chunk.
    pub position: u32,
    /// 1-based index of the bloom in the ritual's sequence, and the sequence
    /// length. For a chunked bloom these do not advance per chunk.
    pub bloom_position: usize,
    pub bloom_total: usize,
    /// The title as the responder saw it, including any `(Part N/M)` suffix.
    pub title_shown: String,
    pub guess: String,
    pub model_id: Option<String>,
    pub phrase_source: String,
    /// Snapshot of the phrase list this guess was matched against. Phrases get
    /// edited over time; without the snapshot an old row cannot be read back.
    pub phrases: Vec<String>,
    pub match_kind: String,
    /// Index into `phrases` of the phrase that matched, if any.
    pub match_index: Option<usize>,
    pub bucket: String,
    /// SHA-256 of the chunk text the guess was made against.
    pub content_hash: String,
}

/// SHA-256 of `text`, lowercase hex.
pub fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{:02x}", byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_lowercase_hex_of_expected_width() {
        let h = content_hash("chunk text");
        assert_eq!(h.len(), 64);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn content_hash_matches_known_sha256_vector() {
        // SHA-256 of the empty string.
        assert_eq!(
            content_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn content_hash_differs_for_different_text() {
        assert_ne!(content_hash("alpha"), content_hash("beta"));
    }
}
