use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::engage::{MatchResult, fuzzy_match};
use crate::knowledge::KnowledgeEntry;
use crate::store::{AgentContext, KnowledgeStore, WakeCascade};
use crate::wake_token::*;

/// Start a new wake ritual session
pub fn begin_ritual(cascade: &WakeCascade) -> Result<String> {
    if cascade.core.is_empty() && cascade.recent.is_empty() && cascade.bridges.is_empty() {
        bail!("No blooms to wake");
    }

    let token = WakeSessionToken::new(cascade);
    let total = token.total();

    // Build lookup map
    let all_blooms = build_bloom_map(cascade);

    // Get first bloom
    let first_id = token
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("No blooms in session"))?;
    let first_bloom = all_blooms
        .get(first_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", first_id))?;

    let response = WakeBeginResponse {
        status: "ritual_started".to_string(),
        session: token.sign()?,
        prompt: BloomPrompt::from(*first_bloom),
        progress: Progress {
            current: 1,
            total,
            remembered: None,
            needed_help: None,
            skipped: None,
        },
    };

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Process a wake phrase response
pub fn respond_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    phrase: &str,
    session_token: &str,
) -> Result<String> {
    let mut token = WakeSessionToken::verify(session_token)?;

    // Fetch blooms by ID from token (source of truth!)
    let all_blooms = fetch_blooms_by_ids(db, ctx, &token.bloom_ids)?;

    // Verify we're on the right bloom
    let expected_id = token
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?;

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id.to_string()),
        };
        return Ok(serde_json::to_string_pretty(&response)?);
    }

    // Get the bloom
    let bloom = all_blooms
        .get(expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    // Get the pre-selected wake phrase from token
    let phrase_idx = token
        .current_phrase_index()
        .ok_or_else(|| anyhow::anyhow!("This bloom has no wake phrase - use --skip instead"))?;

    let wake_phrase = if !bloom.wake_phrases.is_empty() {
        bloom.wake_phrases.get(phrase_idx)
            .ok_or_else(|| anyhow::anyhow!("Invalid phrase index"))?
            .clone()
    } else if let Some(ref phrase) = bloom.wake_phrase {
        phrase.clone()
    } else {
        bail!("This bloom has no wake phrase - use --skip instead");
    };

    // Match the phrase
    let match_result = fuzzy_match(phrase, &wake_phrase);

    match match_result {
        MatchResult::Exact | MatchResult::Close => {
            // SUCCESS! Advance and return bloom
            token.advance_remembered();

            let match_type = if matches!(match_result, MatchResult::Exact) {
                "exact"
            } else {
                "close"
            };

            // Get next bloom if any
            let (next, progress, summary) = get_next_and_progress(&token, &all_blooms)?;

            // Create BloomFull with matched phrase
            let mut bloom_full = BloomFull::from(bloom);
            bloom_full.matched_phrase = Some(wake_phrase.clone());

            let response = WakeRespondResponse {
                status: "remembered".to_string(),
                match_type: Some(match_type.to_string()),
                bloom: Some(bloom_full),
                attempt: None,
                max_attempts: None,
                hint: None,
                prompt: None,
                session: token.sign()?,
                next,
                progress: Some(progress),
                summary,
            };

            Ok(serde_json::to_string_pretty(&response)?)
        }
        MatchResult::Partial | MatchResult::Wrong => {
            // Increment attempt
            token.increment_attempt();
            let attempt = token.attempts_on_current;

            if attempt >= 3 {
                // Max attempts reached - reveal and advance
                token.advance_helped();

                let (next, progress, summary) = get_next_and_progress(&token, &all_blooms)?;

                // Create BloomFull with revealed phrase
                let mut bloom_full = BloomFull::from(bloom);
                bloom_full.matched_phrase = Some(wake_phrase.clone());

                let response = WakeRespondResponse {
                    status: "revealed".to_string(),
                    match_type: None,
                    bloom: Some(bloom_full),
                    attempt: None,
                    max_attempts: None,
                    hint: None,
                    prompt: None,
                    session: token.sign()?,
                    next,
                    progress: Some(progress),
                    summary,
                };

                Ok(serde_json::to_string_pretty(&response)?)
            } else {
                // Give hint and ask for retry
                let hint = generate_hint(&wake_phrase, attempt);

                let response = WakeRespondResponse {
                    status: "incorrect".to_string(),
                    match_type: None,
                    bloom: None,
                    attempt: Some(attempt),
                    max_attempts: Some(3),
                    hint: Some(hint),
                    prompt: Some(BloomPrompt::from(bloom)),
                    session: token.sign()?,
                    next: None,
                    progress: None,
                    summary: None,
                };

                Ok(serde_json::to_string_pretty(&response)?)
            }
        }
    }
}

/// Skip a bloom (for blooms without wake phrase)
pub fn skip_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    session_token: &str,
) -> Result<String> {
    let mut token = WakeSessionToken::verify(session_token)?;

    // Fetch blooms by ID from token (source of truth!)
    let all_blooms = fetch_blooms_by_ids(db, ctx, &token.bloom_ids)?;

    // Verify we're on the right bloom
    let expected_id = token
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?;

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id.to_string()),
        };
        return Ok(serde_json::to_string_pretty(&response)?);
    }

    // Get the bloom
    let bloom = all_blooms
        .get(expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    // Advance as skipped
    token.advance_skipped();

    let (next, progress, summary) = get_next_and_progress(&token, &all_blooms)?;

    let response = WakeSkipResponse {
        status: "skipped".to_string(),
        bloom: BloomFull::from(bloom),
        session: token.sign()?,
        next,
        progress: Some(progress),
        summary,
    };

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Fetch blooms by IDs and build lookup map
fn fetch_blooms_by_ids(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_ids: &[String],
) -> Result<HashMap<String, KnowledgeEntry>> {
    let mut map = HashMap::new();

    for id in bloom_ids {
        if let Some(entry) = db.get(id, ctx)? {
            map.insert(id.clone(), entry);
        } else {
            bail!("Bloom not found in database: {}", id);
        }
    }

    Ok(map)
}

/// Build lookup map of all blooms
fn build_bloom_map(cascade: &WakeCascade) -> HashMap<String, &KnowledgeEntry> {
    let mut map = HashMap::new();

    for entry in &cascade.core {
        map.insert(entry.id.clone(), entry);
    }
    for entry in &cascade.recent {
        map.insert(entry.id.clone(), entry);
    }
    for entry in &cascade.bridges {
        map.insert(entry.id.clone(), entry);
    }

    map
}

/// Get next bloom prompt and current progress
fn get_next_and_progress(
    token: &WakeSessionToken,
    all_blooms: &HashMap<String, KnowledgeEntry>,
) -> Result<(Option<BloomPrompt>, Progress, Option<Summary>)> {
    let current = token.current_position();
    let total = token.total();

    let progress = Progress {
        current,
        total,
        remembered: Some(token.remembered_count),
        needed_help: Some(token.needed_help_count),
        skipped: Some(token.skipped_count),
    };

    if token.is_complete() {
        // Ritual complete
        let summary = Summary {
            total,
            remembered: token.remembered_count,
            needed_help: token.needed_help_count,
            skipped: token.skipped_count,
        };
        Ok((None, progress, Some(summary)))
    } else {
        // Get next bloom
        let next_id = token
            .current_bloom_id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get next bloom"))?;
        let next_bloom = all_blooms
            .get(next_id)
            .ok_or_else(|| anyhow::anyhow!("Next bloom not found: {}", next_id))?;

        Ok((Some(BloomPrompt::from(next_bloom)), progress, None))
    }
}

/// Generate progressive hints
fn generate_hint(phrase: &str, attempt: u8) -> String {
    match attempt {
        1 => {
            // Hint 1: starts with...
            let words: Vec<&str> = phrase.split_whitespace().collect();
            if let Some(first_word) = words.first() {
                format!("starts with \"{}...\"", first_word)
            } else {
                "think carefully...".to_string()
            }
        }
        2 => {
            // Hint 2: blank out middle word
            let words: Vec<&str> = phrase.split_whitespace().collect();
            if words.len() >= 3 {
                let middle_idx = words.len() / 2;
                let hint_words: Vec<String> = words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        if i == middle_idx {
                            "___".to_string()
                        } else {
                            w.to_string()
                        }
                    })
                    .collect();
                format!("\"{}\"", hint_words.join(" "))
            } else if words.len() == 2 {
                format!("\"{} ___\"", words[0])
            } else if !words.is_empty() {
                let first_word = words[0];
                if first_word.len() > 3 {
                    let prefix = &first_word[..3];
                    format!("\"{}...\"", prefix)
                } else {
                    phrase.to_string()
                }
            } else {
                "almost there...".to_string()
            }
        }
        _ => "one more try...".to_string(),
    }
}
