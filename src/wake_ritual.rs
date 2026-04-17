use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::engage::{MatchResult, fuzzy_match};
use crate::knowledge::KnowledgeEntry;
use crate::store::{AgentContext, KnowledgeStore, WakeCascade};
use crate::wake_chunk::{
    ChunkPlan, PhraseMatch, PhraseMode, chunk_threshold, compare_phrase, compute_chunks,
    extract_salient_phrase,
};
use crate::wake_token::*;

/// Which phrase source unlocked a chunk — authored by the bloom owner, or
/// auto-derived from the chunk's own content (§5 of the mx#211 design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhraseSource {
    Authored,
    Derived,
}

impl PhraseSource {
    fn as_str(self) -> &'static str {
        match self {
            PhraseSource::Authored => "authored",
            PhraseSource::Derived => "derived",
        }
    }

    fn mode(self) -> PhraseMode {
        match self {
            PhraseSource::Authored => PhraseMode::Authored,
            PhraseSource::Derived => PhraseMode::Derived,
        }
    }
}

/// Pick the wake phrase for a specific chunk of a bloom. Authored phrases
/// win when available at the given index; beyond the authored count we
/// auto-derive from the chunk's own content (§5.2).
///
/// **P==0 semantics:** for blooms with zero authored phrases we return `None`
/// — those blooms stay skip-type across every chunk (conservative default
/// per the P==0 decision). Never auto-derive for phraseless blooms.
///
/// Returns `(phrase, source)`. `source` drives the comparison tolerance:
/// authored phrases get exact-compare (Authored mode) while derived phrases
/// get softened comparisons (Derived mode).
fn phrase_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u8,
    chunk_total: u8,
    chunk_content: &str,
) -> Option<(String, PhraseSource)> {
    let authored_count = authored_phrase_count(entry);
    if authored_count == 0 {
        // P==0 conservative default — skip-type across all chunks.
        return None;
    }
    if (chunk_idx as usize) < authored_count as usize
        && let Some(p) = authored_phrase_at(entry, chunk_idx as usize)
    {
        return Some((p, PhraseSource::Authored));
    }
    // Chunk beyond the authored count: auto-derive from chunk content.
    Some((
        extract_salient_phrase(chunk_content, chunk_idx, chunk_total),
        PhraseSource::Derived,
    ))
}

/// Compute the sum of chunk counts across all blooms in the session, using
/// the current in-memory content. Eager total for `progress.total` at begin
/// time (§7.1). Cheap: O(N * content_len), microseconds for typical cascades.
fn total_chunks_across_cascade(
    session: &WakeSession,
    blooms: &HashMap<String, KnowledgeEntry>,
) -> usize {
    let threshold = chunk_threshold();
    let mut total: usize = 0;
    for id in &session.bloom_ids {
        if let Some(entry) = blooms.get(id) {
            let content = bloom_content(entry);
            let plan = compute_chunks(&content, threshold);
            total += plan.total as usize;
        } else {
            total += 1; // fallback — treat missing blooms as 1-chunk
        }
    }
    total
}

/// Formatted body or summary or placeholder — the string used for chunking.
fn bloom_content(entry: &KnowledgeEntry) -> String {
    entry
        .body
        .clone()
        .or_else(|| entry.summary.clone())
        .unwrap_or_else(|| "(no content)".to_string())
}

/// Build a chunk-aware `BloomPrompt` for the bloom at the session's current
/// cursor. Decorates the title with `(Part N/M)` server-side so existing
/// CLIs that display the title surface chunk position for free.
///
/// If the chunk plan has only one chunk (i.e. content ≤ threshold), the
/// prompt is identical to the non-chunked `BloomPrompt::from(entry)` —
/// backward-compatible contract.
fn build_prompt_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u8,
    plan: &ChunkPlan,
    content: &str,
) -> BloomPrompt {
    let mut prompt = BloomPrompt::from(entry);
    if plan.total > 1 {
        prompt.title = format!("{} (Part {}/{})", entry.title, chunk_idx + 1, plan.total);
        prompt.chunk = Some(ChunkRef {
            index: chunk_idx + 1,
            total: plan.total,
            oversized: if plan.is_oversized(chunk_idx) {
                Some(true)
            } else {
                None
            },
        });
        // Indicate authored-vs-derived only when there's a phrase for the
        // chunk. P==0 blooms skip; non-P==0 blooms expose the source.
        let chunk_content = plan.chunk(content, chunk_idx);
        if let Some((_, source)) = phrase_for_chunk(entry, chunk_idx, plan.total, chunk_content) {
            prompt.phrase_source = Some(source.as_str().to_string());
        }
    } else {
        // Single-chunk bloom — still surface phrase_source if applicable so
        // consumers have a uniform signal regardless of chunking.
        if authored_phrase_count(entry) > 0 {
            prompt.phrase_source = Some(PhraseSource::Authored.as_str().to_string());
        }
    }
    prompt
}

/// Build a chunk-aware `BloomFull` for the chunk currently being revealed.
/// The `content` field is the *chunk's* content, not the whole bloom — this
/// is the critical behavior change in mx#211.
fn build_full_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u8,
    plan: &ChunkPlan,
    content: &str,
    matched_phrase: Option<String>,
    source: Option<PhraseSource>,
    chunk_truncated: bool,
) -> BloomFull {
    let mut full = BloomFull::from(entry);
    if plan.total > 1 {
        let chunk_content = plan.chunk(content, chunk_idx);
        full.content = chunk_content.to_string();
        full.title = format!("{} (Part {}/{})", entry.title, chunk_idx + 1, plan.total);
        full.chunk = Some(ChunkRef {
            index: chunk_idx + 1,
            total: plan.total,
            oversized: if plan.is_oversized(chunk_idx) {
                Some(true)
            } else {
                None
            },
        });
    }
    // For single-chunk blooms, BloomFull::from already populates the full
    // content. We only override for chunked blooms above.

    full.matched_phrase = matched_phrase;
    full.phrase_source = source.map(|s| s.as_str().to_string());
    if chunk_truncated {
        full.chunk_truncated = Some(true);
    }
    full
}

/// Start a new wake ritual session.
pub fn begin_ritual(db: &dyn KnowledgeStore, cascade: &WakeCascade) -> Result<String> {
    if cascade.core.is_empty() && cascade.recent.is_empty() && cascade.bridges.is_empty() {
        bail!("No blooms to wake");
    }

    let session = WakeSession::new(cascade);

    // Build lookup map from the cascade we already have.
    let owned_blooms: HashMap<String, KnowledgeEntry> = build_bloom_map_owned(cascade);

    // Eager total-chunks count for progress.total.
    let total_steps = total_chunks_across_cascade(&session, &owned_blooms);

    // Get first bloom + its chunk plan.
    let first_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("No blooms in session"))?;
    let first_bloom = owned_blooms
        .get(first_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", first_id))?;
    let first_content = bloom_content(first_bloom);
    let first_plan = compute_chunks(&first_content, chunk_threshold());

    let prompt = build_prompt_for_chunk(first_bloom, 0, &first_plan, &first_content);

    // Persist session to DB.
    let session_id = db.create_wake_session(&session)?;

    // Return signed token at step 0.
    let token = create_token(&session_id, session.step);

    let response = WakeBeginResponse {
        status: "ritual_started".to_string(),
        session: token,
        prompt,
        progress: Progress {
            current: 1,
            total: total_steps.max(1),
            remembered: None,
            needed_help: None,
            skipped: None,
            bloom_current: Some(1),
            bloom_total: Some(session.total_blooms()),
        },
    };

    Ok(serde_json::to_string(&response)?)
}

/// Process a wake phrase response.
pub fn respond_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    phrase: &str,
    token_str: &str,
) -> Result<String> {
    let (session_id, token_step) =
        verify_token(token_str).map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))?;

    let mut session = db
        .get_wake_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    // Anti-replay: token step must match server-side state.
    if session.step != token_step {
        bail!(
            "Token out of sync: token step {} but session at step {}",
            token_step,
            session.step
        );
    }

    let all_blooms = fetch_blooms_by_ids(db, ctx, &session.bloom_ids)?;

    let expected_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?
        .to_string();

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id),
        };
        return Ok(serde_json::to_string(&response)?);
    }

    let bloom = all_blooms
        .get(&expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    let content = bloom_content(bloom);
    let plan = compute_chunks(&content, chunk_threshold());

    // If the bloom shrank past our chunk cursor, advance to next bloom.
    // Flagged via chunk_truncated (§2.2).
    let chunk_truncated = session.clamp_if_chunks_shrank(plan.total);
    if chunk_truncated {
        // Persist the clamp and return a skip-like response so the consumer
        // can see what happened.
        let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;
        if session.is_complete() {
            db.delete_wake_session(&session_id)?;
        } else {
            db.update_wake_session(&session)?;
        }
        let bloom_full =
            build_full_for_chunk(bloom, 0, &plan, &content, None, None, chunk_truncated);
        let new_token = create_token(&session_id, session.step);
        let response = WakeRespondResponse {
            status: "chunk_truncated".to_string(),
            match_type: None,
            bloom: Some(bloom_full),
            attempt: None,
            hint: None,
            prompt: None,
            session: new_token,
            next,
            progress: Some(progress),
            summary,
            content_changed_during_ritual: None,
        };
        return Ok(serde_json::to_string(&response)?);
    }

    let chunk_idx = session.current_chunk_index;
    let chunk_content = plan.chunk(&content, chunk_idx);

    // P==0 bloom? Reject respond path — consumer must --skip.
    let (wake_phrase, source) = match phrase_for_chunk(bloom, chunk_idx, plan.total, chunk_content)
    {
        Some(p) => p,
        None => bail!("This bloom has no wake phrase - use --skip instead"),
    };

    // Compare: first via our tolerant compare_phrase (picks up authored-vs-
    // derived tolerance), then fall through to fuzzy_match for the existing
    // Close/Partial/Wrong tiers so we don't regress the hint flow.
    let tolerant = compare_phrase(phrase, &wake_phrase, source.mode());
    let match_result = match tolerant {
        PhraseMatch::Exact => MatchResult::Exact,
        PhraseMatch::Tolerant => MatchResult::Close,
        PhraseMatch::Mismatch => fuzzy_match(phrase, &wake_phrase),
    };

    match match_result {
        MatchResult::Exact | MatchResult::Close => {
            session.advance_remembered(plan.total);

            let match_type = if matches!(match_result, MatchResult::Exact) {
                "exact"
            } else {
                "close"
            };

            let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

            if session.is_complete() {
                db.delete_wake_session(&session_id)?;
            } else {
                db.update_wake_session(&session)?;
            }

            let bloom_full = build_full_for_chunk(
                bloom,
                chunk_idx,
                &plan,
                &content,
                Some(wake_phrase.clone()),
                Some(source),
                false,
            );

            let new_token = create_token(&session_id, session.step);

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
                content_changed_during_ritual: None,
            };

            Ok(serde_json::to_string(&response)?)
        }
        MatchResult::Partial | MatchResult::Wrong => {
            session.increment_attempt();
            let attempt = session.attempts_on_current;

            if attempt >= 3 {
                session.advance_helped(plan.total);

                let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

                if session.is_complete() {
                    db.delete_wake_session(&session_id)?;
                } else {
                    db.update_wake_session(&session)?;
                }

                let bloom_full = build_full_for_chunk(
                    bloom,
                    chunk_idx,
                    &plan,
                    &content,
                    Some(wake_phrase.clone()),
                    Some(source),
                    false,
                );

                let new_token = create_token(&session_id, session.step);

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
                    content_changed_during_ritual: None,
                };

                Ok(serde_json::to_string(&response)?)
            } else {
                db.update_wake_session(&session)?;

                let hint = generate_hint(&wake_phrase, attempt);

                // Same step (retry), fresh token.
                let new_token = create_token(&session_id, session.step);

                // Risk 9 diagnostic: if a derived phrase rejected, surface
                // content_changed_during_ritual as an advisory. Consumers
                // can use this to suggest a `--begin` restart when mid-ritual
                // edits may have shifted the derived phrase out from under
                // them. Best-effort: we can't cleanly distinguish "user typed
                // wrong" from "content changed" without persisting extra
                // state, which the design deliberately avoids (§6, §10 Risk 9).
                let content_changed = source == PhraseSource::Derived;

                let response = WakeRespondResponse {
                    status: "incorrect".to_string(),
                    match_type: None,
                    bloom: None,
                    attempt: Some(attempt),
                    hint: Some(hint),
                    prompt: Some(build_prompt_for_chunk(bloom, chunk_idx, &plan, &content)),
                    session: new_token,
                    next: None,
                    progress: None,
                    summary: None,
                    content_changed_during_ritual: if content_changed { Some(true) } else { None },
                };

                Ok(serde_json::to_string(&response)?)
            }
        }
    }
}

/// Skip a bloom chunk (for phraseless blooms or consumer-initiated skip).
pub fn skip_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    token_str: &str,
) -> Result<String> {
    let (session_id, token_step) =
        verify_token(token_str).map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))?;

    let mut session = db
        .get_wake_session(&session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    if session.step != token_step {
        bail!(
            "Token out of sync: token step {} but session at step {}",
            token_step,
            session.step
        );
    }

    let all_blooms = fetch_blooms_by_ids(db, ctx, &session.bloom_ids)?;

    let expected_id = session
        .current_bloom_id()
        .ok_or_else(|| anyhow::anyhow!("Ritual already complete"))?
        .to_string();

    if bloom_id != expected_id {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "invalid_bloom_id".to_string(),
            message: format!("Expected bloom {}, got {}", expected_id, bloom_id),
            expected_id: Some(expected_id),
        };
        return Ok(serde_json::to_string(&response)?);
    }

    let bloom = all_blooms
        .get(&expected_id)
        .ok_or_else(|| anyhow::anyhow!("Bloom not found: {}", expected_id))?;

    let content = bloom_content(bloom);
    let plan = compute_chunks(&content, chunk_threshold());
    let chunk_truncated = session.clamp_if_chunks_shrank(plan.total);

    // Skip advances past exactly one chunk (not the whole bloom if chunked).
    // For P==0 blooms the consumer calls --skip K times to walk through all K
    // chunks — expected behavior per §5.9.
    let chunk_idx = session.current_chunk_index;
    session.advance_skipped(plan.total);

    let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

    if session.is_complete() {
        db.delete_wake_session(&session_id)?;
    } else {
        db.update_wake_session(&session)?;
    }

    let new_token = create_token(&session_id, session.step);

    let response = WakeSkipResponse {
        status: "skipped".to_string(),
        bloom: build_full_for_chunk(
            bloom,
            chunk_idx,
            &plan,
            &content,
            None,
            None,
            chunk_truncated,
        ),
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

/// Build owned lookup map of all blooms from cascade.
fn build_bloom_map_owned(cascade: &WakeCascade) -> HashMap<String, KnowledgeEntry> {
    let mut map = HashMap::new();

    for entry in &cascade.core {
        map.insert(entry.id.clone(), entry.clone());
    }
    for entry in &cascade.recent {
        map.insert(entry.id.clone(), entry.clone());
    }
    for entry in &cascade.bridges {
        map.insert(entry.id.clone(), entry.clone());
    }

    map
}

/// Get next bloom prompt and current progress. Handles both in-bloom chunk
/// advancement (staying on the same bloom) and cross-bloom advancement.
fn get_next_and_progress(
    session: &WakeSession,
    all_blooms: &HashMap<String, KnowledgeEntry>,
) -> Result<(Option<BloomPrompt>, Progress, Option<Summary>)> {
    // `step` is 1-indexed for display. After an advance, session.step is the
    // count of chunks already walked; display shows "we're on chunk step+1".
    let display_current = session.step as usize + 1;

    // Re-compute total chunks for progress (cheap; keeps the total fresh for
    // mid-ritual edits per §7.1).
    let total_chunks = total_chunks_across_cascade(session, all_blooms).max(1);
    let bloom_current = session.current_bloom_position().min(session.total_blooms());

    let progress = Progress {
        current: display_current,
        total: total_chunks,
        remembered: Some(session.remembered_count),
        needed_help: Some(session.needed_help_count),
        skipped: Some(session.skipped_count),
        bloom_current: Some(bloom_current),
        bloom_total: Some(session.total_blooms()),
    };

    if session.is_complete() {
        let summary = Summary {
            total: session.step as usize,
            remembered: session.remembered_count,
            needed_help: session.needed_help_count,
            skipped: session.skipped_count,
            blooms_complete: None, // populated in PR 3
            chunks_remembered: Some(session.remembered_count),
            chunks_needed_help: Some(session.needed_help_count),
            chunks_skipped: Some(session.skipped_count),
        };
        Ok((None, progress, Some(summary)))
    } else {
        let next_id = session
            .current_bloom_id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get next bloom"))?;
        let next_bloom = all_blooms
            .get(next_id)
            .ok_or_else(|| anyhow::anyhow!("Next bloom not found: {}", next_id))?;

        let next_content = bloom_content(next_bloom);
        let next_plan = compute_chunks(&next_content, chunk_threshold());
        let next_chunk_idx = session.current_chunk_index;

        Ok((
            Some(build_prompt_for_chunk(
                next_bloom,
                next_chunk_idx,
                &next_plan,
                &next_content,
            )),
            progress,
            None,
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    // =====================================================================
    // Regression tests for unicode boundary panic fix (PR #162)
    //
    // generate_hint() previously used `&first_word[..3]` (byte-index slicing)
    // on single-word wake phrases. Multi-byte UTF-8 characters at the start
    // of the word would cause a panic when byte index 3 landed inside a
    // character. The fix uses `.chars().take(3).collect()` instead.
    // =====================================================================

    #[test]
    fn test_generate_hint_single_emoji_word_would_panic() {
        let phrase = "\u{1F41F}\u{1F41F}\u{1F41F}\u{1F41F}\u{1F41F}";
        assert_eq!(phrase.chars().count(), 5);
        assert!(!phrase.is_char_boundary(3));

        let result = generate_hint(phrase, 2);
        let expected_prefix: String = phrase.chars().take(3).collect();
        assert!(result.contains(&expected_prefix));
        assert!(result.contains("..."));
    }

    #[test]
    fn test_generate_hint_single_cjk_word_would_panic() {
        let phrase = "\u{4E16}\u{754C}\u{4F60}\u{597D}\u{5417}";
        assert_eq!(phrase.chars().count(), 5);

        let result = generate_hint(phrase, 2);
        let expected_prefix: String = phrase.chars().take(3).collect();
        assert!(result.contains(&expected_prefix));
    }

    #[test]
    fn test_generate_hint_single_mixed_multibyte_word_would_panic() {
        let phrase = "\u{00E9}\u{00E9}\u{00E9}\u{00E9}";
        assert_eq!(phrase.chars().count(), 4);
        assert_eq!(phrase.len(), 8);
        assert!(!phrase.is_char_boundary(3));

        let result = generate_hint(phrase, 2);
        let expected_prefix: String = phrase.chars().take(3).collect();
        assert!(result.contains(&expected_prefix));
    }

    #[test]
    fn test_generate_hint_attempt_1_first_word_with_emoji() {
        let phrase = "\u{1F41F}\u{1F41F} hello world";
        let result = generate_hint(phrase, 1);
        assert!(result.contains("\u{1F41F}\u{1F41F}"));
        assert!(result.starts_with("starts with"));
    }

    #[test]
    fn test_generate_hint_attempt_2_multiword_with_emoji() {
        let phrase = "\u{1F41F}\u{1F41F} middle \u{4E16}\u{754C}";
        let result = generate_hint(phrase, 2);
        assert!(result.contains("___"));
        assert!(result.contains("\u{1F41F}\u{1F41F}"));
        assert!(result.contains("\u{4E16}\u{754C}"));
    }

    #[test]
    fn test_generate_hint_attempt_2_two_emoji_words() {
        let phrase = "\u{1F41F}\u{1F41F} \u{4E16}\u{754C}";
        let result = generate_hint(phrase, 2);
        assert!(result.contains("\u{1F41F}\u{1F41F}"));
        assert!(result.contains("___"));
    }

    #[test]
    fn test_generate_hint_short_single_emoji_word() {
        let phrase = "\u{1F41F}\u{1F41F}";
        assert_eq!(phrase.chars().count(), 2);

        let result = generate_hint(phrase, 2);
        assert_eq!(result, phrase);
    }

    // =====================================================================
    // phrase_for_chunk unit tests — authored-then-sampled selector logic
    // =====================================================================

    fn test_entry() -> KnowledgeEntry {
        // KnowledgeEntry has no Default; use serde_json round-trip to
        // construct a minimal valid entry (all fields have #[serde(default)]
        // except id/title/category_id).
        serde_json::from_str::<KnowledgeEntry>(
            r#"{"id":"kn-test","category_id":"bloom","title":"Test","body":"body"}"#,
        )
        .expect("test entry deserialize")
    }

    fn entry_with_phrases(phrases: Vec<&str>) -> KnowledgeEntry {
        let mut e = test_entry();
        e.wake_phrases = phrases.into_iter().map(|s| s.to_string()).collect();
        e
    }

    #[test]
    fn phrase_for_chunk_authored_within_count() {
        let e = entry_with_phrases(vec!["alpha", "beta", "gamma"]);
        let (p, src) = phrase_for_chunk(&e, 0, 5, "chunk 0 content").unwrap();
        assert_eq!(p, "alpha");
        assert_eq!(src, PhraseSource::Authored);

        let (p, src) = phrase_for_chunk(&e, 2, 5, "chunk 2 content").unwrap();
        assert_eq!(p, "gamma");
        assert_eq!(src, PhraseSource::Authored);
    }

    #[test]
    fn phrase_for_chunk_derived_beyond_count() {
        let e = entry_with_phrases(vec!["alpha"]);
        let chunk = "\n## Derived heading here\n\nbody text";
        let (p, src) = phrase_for_chunk(&e, 3, 5, chunk).unwrap();
        assert_eq!(p, "Derived heading here");
        assert_eq!(src, PhraseSource::Derived);
    }

    #[test]
    fn phrase_for_chunk_phraseless_returns_none() {
        let e = entry_with_phrases(vec![]);
        assert!(phrase_for_chunk(&e, 0, 3, "content").is_none());
        assert!(phrase_for_chunk(&e, 2, 3, "content").is_none());
    }

    #[test]
    fn phrase_for_chunk_legacy_single_phrase() {
        let mut e = test_entry();
        e.wake_phrase = Some("legacy phrase".to_string());
        let (p, src) = phrase_for_chunk(&e, 0, 1, "chunk").unwrap();
        assert_eq!(p, "legacy phrase");
        assert_eq!(src, PhraseSource::Authored);
    }

    // =====================================================================
    // WakeSession state-machine tests (Risk 4 — off-by-one is the worst
    // failure mode here; assert every transition).
    // =====================================================================

    fn test_cascade(entries: Vec<KnowledgeEntry>) -> WakeCascade {
        WakeCascade {
            core: entries,
            recent: Vec::new(),
            bridges: Vec::new(),
        }
    }

    #[test]
    fn session_new_initializes_both_cursors_to_zero() {
        let cascade = test_cascade(vec![test_entry()]);
        let session = WakeSession::new(&cascade);
        assert_eq!(session.current_index, 0);
        assert_eq!(session.current_chunk_index, 0);
        assert_eq!(session.step, 0);
        assert_eq!(session.total_blooms(), 1);
    }

    #[test]
    fn session_advance_within_bloom_chunks_ticks_chunk_cursor() {
        let mut session = WakeSession::new(&test_cascade(vec![test_entry()]));
        session.advance_remembered(3); // 3-chunk bloom, chunk 0 → 1
        assert_eq!(session.current_index, 0);
        assert_eq!(session.current_chunk_index, 1);
        assert_eq!(session.step, 1);
        assert_eq!(session.remembered_count, 1);

        session.advance_remembered(3); // chunk 1 → 2
        assert_eq!(session.current_index, 0);
        assert_eq!(session.current_chunk_index, 2);
        assert_eq!(session.step, 2);

        session.advance_remembered(3); // chunk 2 → next bloom
        assert_eq!(session.current_index, 1);
        assert_eq!(session.current_chunk_index, 0);
        assert_eq!(session.step, 3);
    }

    #[test]
    fn session_step_monotonic_across_bloom_and_chunk_advances() {
        let mut session = WakeSession::new(&test_cascade(vec![
            test_entry(),
            test_entry(),
            test_entry(),
        ]));
        // Bloom 0: 3 chunks
        session.advance_remembered(3);
        session.advance_remembered(3);
        session.advance_remembered(3);
        // Bloom 1: 1 chunk (not chunked)
        session.advance_skipped(1);
        // Bloom 2: 2 chunks
        session.advance_helped(2);
        session.advance_helped(2);
        assert_eq!(session.step, 6);
        assert_eq!(session.remembered_count, 3);
        assert_eq!(session.needed_help_count, 2);
        assert_eq!(session.skipped_count, 1);
        assert!(session.is_complete());
    }

    #[test]
    fn session_non_chunked_bloom_advances_immediately() {
        let mut session = WakeSession::new(&test_cascade(vec![test_entry(), test_entry()]));
        session.advance_remembered(1); // single-chunk bloom
        assert_eq!(session.current_index, 1);
        assert_eq!(session.current_chunk_index, 0);
    }

    #[test]
    fn session_clamp_advances_when_bloom_shrank() {
        let mut session = WakeSession::new(&test_cascade(vec![test_entry(), test_entry()]));
        session.current_chunk_index = 4; // pretend we were on chunk 4 of 5
        let clamped = session.clamp_if_chunks_shrank(2); // bloom now has 2 chunks
        assert!(clamped);
        assert_eq!(session.current_index, 1);
        assert_eq!(session.current_chunk_index, 0);
    }

    #[test]
    fn session_clamp_noop_when_cursor_in_range() {
        let mut session = WakeSession::new(&test_cascade(vec![test_entry()]));
        session.current_chunk_index = 1;
        let clamped = session.clamp_if_chunks_shrank(3);
        assert!(!clamped);
        assert_eq!(session.current_chunk_index, 1);
        assert_eq!(session.current_index, 0);
    }

    #[test]
    fn session_phraseless_bloom_meta() {
        let cascade = test_cascade(vec![test_entry()]); // no wake_phrases
        let session = WakeSession::new(&cascade);
        let meta = session.current_meta().unwrap();
        assert_eq!(meta.authored_phrase_count, 0);
        assert!(meta.is_phraseless);
    }

    #[test]
    fn session_authored_phrase_count_respects_wake_phrases() {
        let mut e = test_entry();
        e.wake_phrases = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let cascade = test_cascade(vec![e]);
        let session = WakeSession::new(&cascade);
        let meta = session.current_meta().unwrap();
        assert_eq!(meta.authored_phrase_count, 3);
        assert!(!meta.is_phraseless);
    }

    // =====================================================================
    // End-to-end ritual walk with a >30KB bloom (realistic Ops scenario).
    // Uses the actual compute_chunks + phrase_for_chunk + advance logic.
    // =====================================================================

    fn make_large_bloom(target_bytes: usize, phrases: Vec<&str>) -> KnowledgeEntry {
        // Build realistic markdown content with H2 sections so the chunker
        // has semantic break points to prefer over the UTF-8 fallback.
        let mut body = String::new();
        let mut section = 0;
        while body.len() < target_bytes {
            section += 1;
            body.push_str(&format!(
                "\n## Section {section}\n\n\
                 This is section {section} of the ops bloom. It contains \
                 enough text that multiple sections will cross the chunking \
                 threshold. The wake ritual should walk each chunk in turn \
                 and verify phrases at each boundary.\n\n\
                 - bullet one for section {section}\n\
                 - bullet two for section {section}\n\
                 - bullet three for section {section}\n\n"
            ));
        }
        let mut e = test_entry();
        e.title = "Ops".to_string();
        e.body = Some(body);
        e.wake_phrases = phrases.into_iter().map(|s| s.to_string()).collect();
        e
    }

    #[test]
    fn large_bloom_splits_into_multiple_chunks() {
        let entry = make_large_bloom(69_000, vec!["alpha", "beta", "gamma"]);
        let content = bloom_content(&entry);
        let plan = compute_chunks(&content, 28_000);
        assert!(
            plan.total >= 3,
            "expected ≥3 chunks for 69KB, got {}",
            plan.total
        );
        // Every chunk must be under threshold (no oversized code blocks here).
        for (_, chunk, oversized) in plan.iter(&content) {
            if !oversized {
                assert!(chunk.len() <= 28_000);
            }
        }
    }

    #[test]
    fn large_bloom_authored_then_derived_phrase_sequence() {
        // P=3 authored phrases, K=5 chunks → chunks 0-2 authored, 3-4 derived.
        let entry = make_large_bloom(110_000, vec!["alpha", "beta", "gamma"]);
        let content = bloom_content(&entry);
        let plan = compute_chunks(&content, 28_000);
        assert!(
            plan.total >= 4,
            "need at least 4 chunks, got {}",
            plan.total
        );

        // Authored chunks.
        let (p0, src0) = phrase_for_chunk(&entry, 0, plan.total, plan.chunk(&content, 0)).unwrap();
        assert_eq!(p0, "alpha");
        assert_eq!(src0, PhraseSource::Authored);

        let (p1, src1) = phrase_for_chunk(&entry, 1, plan.total, plan.chunk(&content, 1)).unwrap();
        assert_eq!(p1, "beta");
        assert_eq!(src1, PhraseSource::Authored);

        let (p2, src2) = phrase_for_chunk(&entry, 2, plan.total, plan.chunk(&content, 2)).unwrap();
        assert_eq!(p2, "gamma");
        assert_eq!(src2, PhraseSource::Authored);

        // Derived chunks — should extract from the chunk's own content
        // (markdown heading or first sentence).
        let chunk3 = plan.chunk(&content, 3);
        let (p3, src3) = phrase_for_chunk(&entry, 3, plan.total, chunk3).unwrap();
        assert!(!p3.is_empty());
        assert_eq!(src3, PhraseSource::Derived);
    }

    #[test]
    fn phraseless_large_bloom_returns_none_for_every_chunk() {
        // P==0: all chunks are skip-type per the conservative default.
        let entry = make_large_bloom(90_000, vec![]);
        let content = bloom_content(&entry);
        let plan = compute_chunks(&content, 28_000);
        assert!(plan.total >= 3);
        for idx in 0..plan.total {
            let chunk = plan.chunk(&content, idx);
            let result = phrase_for_chunk(&entry, idx, plan.total, chunk);
            assert!(
                result.is_none(),
                "P==0 bloom should never auto-derive phrases (chunk {})",
                idx
            );
        }
    }

    #[test]
    fn full_ritual_walk_through_large_bloom_advances_all_chunks() {
        let entry = make_large_bloom(85_000, vec!["alpha", "beta"]);
        let content = bloom_content(&entry);
        let plan = compute_chunks(&content, 28_000);
        let total_chunks = plan.total;
        assert!(total_chunks >= 3);

        let mut session = WakeSession::new(&test_cascade(vec![entry]));

        // Walk through every chunk as "remembered". Each advance_remembered
        // call must stay on the bloom until we've walked all chunks, then
        // roll over to the next (non-existent) bloom → ritual complete.
        for expected_chunk in 0..total_chunks {
            assert_eq!(session.current_chunk_index, expected_chunk);
            assert_eq!(session.current_index, 0);
            session.advance_remembered(total_chunks);
        }
        assert!(session.is_complete());
        assert_eq!(session.step, total_chunks as u32);
        assert_eq!(session.remembered_count, total_chunks as u32);
    }

    #[test]
    fn derived_phrase_tolerant_match_accepts_case_and_punct_variants() {
        use crate::wake_chunk::{PhraseMatch, PhraseMode, compare_phrase};

        let entry = make_large_bloom(85_000, vec!["alpha"]);
        let content = bloom_content(&entry);
        let plan = compute_chunks(&content, 28_000);
        assert!(plan.total >= 2);

        // Chunk 1 uses a derived phrase. Grab it.
        let chunk1 = plan.chunk(&content, 1);
        let (target, src) = phrase_for_chunk(&entry, 1, plan.total, chunk1).unwrap();
        assert_eq!(src, PhraseSource::Derived);

        // Same phrase, lowercased and with trailing period — should match.
        let variant = format!("{}.", target.to_lowercase());
        let result = compare_phrase(&variant, &target, PhraseMode::Derived);
        assert!(
            matches!(result, PhraseMatch::Exact | PhraseMatch::Tolerant),
            "derived compare should accept case+punct drift: {:?} vs {:?}",
            variant,
            target
        );
    }
}
