use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::engage::{MatchResult, fuzzy_match};
use crate::knowledge::KnowledgeEntry;
use crate::store::{AgentContext, KnowledgeStore, WakeCascade};
use crate::wake_token::*;

/// Start a new wake ritual session
pub fn begin_ritual(db: &dyn KnowledgeStore, cascade: &WakeCascade) -> Result<String> {
    if cascade.core.is_empty() && cascade.recent.is_empty() && cascade.bridges.is_empty() {
        bail!("No blooms to wake");
    }

    let session = WakeSession::new(cascade);
    let total = session.total();

    // Build lookup map from the cascade we already have
    let all_blooms = build_bloom_map(cascade);

    // Get first bloom
    let first_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("No blooms in session"))?;
    let first_bloom = all_blooms
        .get(first_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", first_id))?;

    // Persist session to DB, get back the session_id
    let session_id = db.create_wake_session(&session)?;

    // Return signed token instead of bare session_id
    let token = create_token(&session_id, 0);

    let response = WakeBeginResponse {
        status: "ritual_started".to_string(),
        session: token,
        prompt: BloomPrompt::from(*first_bloom),
        progress: Progress {
            current: 1,
            total,
            remembered: None,
            needed_help: None,
            skipped: None,
        },
    };

    Ok(serde_json::to_string(&response)?)
}

/// Process a wake phrase response
pub fn respond_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    phrase: &str,
    token_str: &str,
) -> Result<String> {
    // Verify token and extract session_id + step
    let (session_id, token_index) =
        verify_token(token_str).map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))?;

    // Load session from DB
    let mut session = db
        .get_wake_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    // Anti-replay: token step must match server-side state
    if session.current_index != token_index {
        bail!(
            "Token out of sync: token step {} but session at step {}",
            token_index,
            session.current_index
        );
    }

    // Fetch blooms by ID from session (source of truth)
    let all_blooms = fetch_blooms_by_ids(db, ctx, &session.bloom_ids)?;

    // Verify we're on the right bloom
    let expected_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?;

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id.to_string()),
        };
        return Ok(serde_json::to_string(&response)?);
    }

    // Get the bloom
    let bloom = all_blooms
        .get(expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    // Get the pre-selected wake phrase from session
    let phrase_idx = session
        .current_phrase_index()
        .ok_or_else(|| anyhow::anyhow!("This bloom has no wake phrase - use --skip instead"))?;

    let wake_phrase = if !bloom.wake_phrases.is_empty() {
        bloom
            .wake_phrases
            .get(phrase_idx)
            .ok_or_else(|| anyhow::anyhow!("Invalid phrase index"))?
            .clone()
    } else if let Some(ref p) = bloom.wake_phrase {
        p.clone()
    } else {
        bail!("This bloom has no wake phrase - use --skip instead");
    };

    // Match the phrase
    let match_result = fuzzy_match(phrase, &wake_phrase);

    match match_result {
        MatchResult::Exact | MatchResult::Close => {
            // SUCCESS! Advance and return bloom
            session.advance_remembered();

            let match_type = if matches!(match_result, MatchResult::Exact) {
                "exact"
            } else {
                "close"
            };

            // Get next bloom if any
            let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

            // Persist updated session (or delete if complete)
            if session.is_complete() {
                db.delete_wake_session(&session_id)?;
            } else {
                db.update_wake_session(&session)?;
            }

            // Create BloomFull with matched phrase
            let mut bloom_full = BloomFull::from(bloom);
            bloom_full.matched_phrase = Some(wake_phrase.clone());

            // New token reflects updated step
            let new_token = create_token(&session_id, session.current_index);

            let response = WakeRespondResponse {
                status: "remembered".to_string(),
                match_type: Some(match_type.to_string()),
                bloom: Some(bloom_full),
                attempt: None,
                hint: None,
                prompt: None,
                session: new_token,
                next,
                progress: Some(progress),
                summary,
            };

            Ok(serde_json::to_string(&response)?)
        }
        MatchResult::Partial | MatchResult::Wrong => {
            // Increment attempt
            session.increment_attempt();
            let attempt = session.attempts_on_current;

            if attempt >= 3 {
                // Max attempts reached - reveal and advance
                session.advance_helped();

                let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

                // Persist updated session (or delete if complete)
                if session.is_complete() {
                    db.delete_wake_session(&session_id)?;
                } else {
                    db.update_wake_session(&session)?;
                }

                // Create BloomFull with revealed phrase
                let mut bloom_full = BloomFull::from(bloom);
                bloom_full.matched_phrase = Some(wake_phrase.clone());

                // New token reflects updated step
                let new_token = create_token(&session_id, session.current_index);

                let response = WakeRespondResponse {
                    status: "revealed".to_string(),
                    match_type: None,
                    bloom: Some(bloom_full),
                    attempt: None,
                    hint: None,
                    prompt: None,
                    session: new_token,
                    next,
                    progress: Some(progress),
                    summary,
                };

                Ok(serde_json::to_string(&response)?)
            } else {
                // Give hint and ask for retry - save incremented attempt count
                db.update_wake_session(&session)?;

                let hint = generate_hint(&wake_phrase, attempt);

                // Same step (retry), but fresh token
                let new_token = create_token(&session_id, session.current_index);

                let response = WakeRespondResponse {
                    status: "incorrect".to_string(),
                    match_type: None,
                    bloom: None,
                    attempt: Some(attempt),
                    hint: Some(hint),
                    prompt: Some(BloomPrompt::from(bloom)),
                    session: new_token,
                    next: None,
                    progress: None,
                    summary: None,
                };

                Ok(serde_json::to_string(&response)?)
            }
        }
    }
}

/// Skip a bloom (for blooms without wake phrase)
pub fn skip_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    token_str: &str,
) -> Result<String> {
    // Verify token and extract session_id + step
    let (session_id, token_index) =
        verify_token(token_str).map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))?;

    // Load session from DB
    let mut session = db
        .get_wake_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    // Anti-replay: token step must match server-side state
    if session.current_index != token_index {
        bail!(
            "Token out of sync: token step {} but session at step {}",
            token_index,
            session.current_index
        );
    }

    // Fetch blooms by ID from session (source of truth)
    let all_blooms = fetch_blooms_by_ids(db, ctx, &session.bloom_ids)?;

    // Verify we're on the right bloom
    let expected_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?;

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id.to_string()),
        };
        return Ok(serde_json::to_string(&response)?);
    }

    // Get the bloom
    let bloom = all_blooms
        .get(expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    // Advance as skipped
    session.advance_skipped();

    let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

    // Persist updated session (or delete if complete)
    if session.is_complete() {
        db.delete_wake_session(&session_id)?;
    } else {
        db.update_wake_session(&session)?;
    }

    // New token reflects updated step
    let new_token = create_token(&session_id, session.current_index);

    let response = WakeSkipResponse {
        status: "skipped".to_string(),
        bloom: BloomFull::from(bloom),
        session: new_token,
        next,
        progress: Some(progress),
        summary,
    };

    Ok(serde_json::to_string(&response)?)
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

/// Build lookup map of all blooms from cascade
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
    session: &WakeSession,
    all_blooms: &HashMap<String, KnowledgeEntry>,
) -> Result<(Option<BloomPrompt>, Progress, Option<Summary>)> {
    let current = session.current_position();
    let total = session.total();

    let progress = Progress {
        current,
        total,
        remembered: Some(session.remembered_count),
        needed_help: Some(session.needed_help_count),
        skipped: Some(session.skipped_count),
    };

    if session.is_complete() {
        // Ritual complete
        let summary = Summary {
            total,
            remembered: session.remembered_count,
            needed_help: session.needed_help_count,
            skipped: session.skipped_count,
        };
        Ok((None, progress, Some(summary)))
    } else {
        // Get next bloom
        let next_id = session
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
                if first_word.chars().count() > 3 {
                    let prefix: String = first_word.chars().take(3).collect();
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
