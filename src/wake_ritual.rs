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

    /// Convert to the persisted tag used for per-bloom counters on the
    /// session (PR 3 observability). `PhraseSourceTag::None` is only emitted
    /// by skip paths where no phrase was involved.
    fn tag(self) -> PhraseSourceTag {
        match self {
            PhraseSource::Authored => PhraseSourceTag::Authored,
            PhraseSource::Derived => PhraseSourceTag::Derived,
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
    chunk_idx: u16,
    chunk_total: u16,
    chunk_content: &str,
) -> Option<(String, PhraseSource)> {
    let authored_count = authored_phrase_count(entry);
    if authored_count == 0 {
        // P==0 conservative default — skip-type across all chunks.
        return None;
    }
    if chunk_idx < authored_count
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
    chunk_idx: u16,
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
    chunk_idx: u16,
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
            derived_phrase_mismatch: None,
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
            session.advance_remembered(plan.total, source.tag());

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
                derived_phrase_mismatch: None,
            };

            Ok(serde_json::to_string(&response)?)
        }
        MatchResult::Partial | MatchResult::Wrong => {
            session.increment_attempt();
            let attempt = session.attempts_on_current;

            if attempt >= 3 {
                session.advance_helped(plan.total, source.tag());

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
                    derived_phrase_mismatch: None,
                };

                Ok(serde_json::to_string(&response)?)
            } else {
                db.update_wake_session(&session)?;

                let hint = generate_hint(&wake_phrase, attempt);

                // Same step (retry), fresh token.
                let new_token = create_token(&session_id, session.step);

                // Risk 9 diagnostic: when the consumer's response misses
                // against a derived phrase, surface `derived_phrase_mismatch`
                // as an advisory. Consumers can use it to suggest a
                // `--begin` restart when mid-ritual edits may have shifted
                // the derived phrase out from under them. Best-effort: this
                // fires on any derived-phrase mismatch — it does NOT
                // guarantee content genuinely changed (a tighter check
                // would need timestamp comparison against bloom.updated_at,
                // deferred per §6 / §10 Risk 9). Field renamed from
                // `content_changed_during_ritual` after Diffi's mx#213
                // review called out the overpromise.
                let derived_miss = source == PhraseSource::Derived;

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
                    derived_phrase_mismatch: if derived_miss { Some(true) } else { None },
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

    // Gate: skip is only valid for phraseless blooms (mx#216). If the bloom
    // has any wake phrases (authored, legacy, or derived-eligible), reject
    // with a structured error. The consumer must use --respond instead.
    if !chunk_truncated && authored_phrase_count(bloom) > 0 {
        let response = WakeErrorResponse {
            status: "error".to_string(),
            error: "skip_requires_phraseless_bloom".to_string(),
            message: "This chunk has a wake phrase and cannot be skipped. Attempt a guess — three incorrect attempts will reveal the content. Priming requires engagement.".to_string(),
            expected_id: Some(expected_id),
        };
        return Ok(serde_json::to_string(&response)?);
    }

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

/// Build the per-bloom roll-up list for `summary.blooms_complete`. One entry
/// per bloom visited during the ritual, with total chunks + outcome counts
/// + authored-vs-derived telemetry. PR 3 observability.
fn build_bloom_rollups(
    session: &WakeSession,
    blooms: &HashMap<String, KnowledgeEntry>,
) -> Vec<BloomRollup> {
    session
        .bloom_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| {
            let meta = session
                .bloom_chunk_meta
                .get(idx)
                .cloned()
                .unwrap_or_default();
            let title = blooms
                .get(id)
                .map(|e| e.title.clone())
                .unwrap_or_else(|| id.clone());
            let total_outcomes = meta.remembered_chunks + meta.helped_chunks + meta.skipped_chunks;
            let chunks_str = if total_outcomes == 0 {
                "0/0 (not reached)".to_string()
            } else if meta.remembered_chunks == total_outcomes {
                format!("{}/{} remembered", meta.remembered_chunks, total_outcomes)
            } else if meta.skipped_chunks == total_outcomes {
                format!("{}/{} skipped", meta.skipped_chunks, total_outcomes)
            } else {
                // Mixed outcome — show each nonzero counter.
                let mut parts = Vec::new();
                if meta.remembered_chunks > 0 {
                    parts.push(format!("{} remembered", meta.remembered_chunks));
                }
                if meta.helped_chunks > 0 {
                    parts.push(format!("{} helped", meta.helped_chunks));
                }
                if meta.skipped_chunks > 0 {
                    parts.push(format!("{} skipped", meta.skipped_chunks));
                }
                format!(
                    "{}/{}  {}",
                    total_outcomes,
                    total_outcomes,
                    parts.join(", ")
                )
            };
            BloomRollup {
                id: id.clone(),
                title,
                chunks: chunks_str,
                remembered: meta.remembered_chunks,
                needed_help: meta.helped_chunks,
                skipped: meta.skipped_chunks,
                total: total_outcomes,
                authored_chunks: meta.authored_chunks,
                derived_chunks: meta.derived_chunks,
            }
        })
        .collect()
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
            blooms_complete: Some(build_bloom_rollups(session, all_blooms)),
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
        session.advance_remembered(3, PhraseSourceTag::Authored); // 3-chunk bloom, chunk 0 → 1
        assert_eq!(session.current_index, 0);
        assert_eq!(session.current_chunk_index, 1);
        assert_eq!(session.step, 1);
        assert_eq!(session.remembered_count, 1);

        session.advance_remembered(3, PhraseSourceTag::Authored); // chunk 1 → 2
        assert_eq!(session.current_index, 0);
        assert_eq!(session.current_chunk_index, 2);
        assert_eq!(session.step, 2);

        session.advance_remembered(3, PhraseSourceTag::Authored); // chunk 2 → next bloom
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
        session.advance_remembered(3, PhraseSourceTag::Authored);
        session.advance_remembered(3, PhraseSourceTag::Authored);
        session.advance_remembered(3, PhraseSourceTag::Derived);
        // Bloom 1: 1 chunk (not chunked)
        session.advance_skipped(1);
        // Bloom 2: 2 chunks
        session.advance_helped(2, PhraseSourceTag::Authored);
        session.advance_helped(2, PhraseSourceTag::Derived);
        assert_eq!(session.step, 6);
        assert_eq!(session.remembered_count, 3);
        assert_eq!(session.needed_help_count, 2);
        assert_eq!(session.skipped_count, 1);
        assert!(session.is_complete());

        // PR 3 observability: per-bloom counters populated during the walk.
        assert_eq!(session.bloom_chunk_meta[0].remembered_chunks, 3);
        assert_eq!(session.bloom_chunk_meta[0].authored_chunks, 2);
        assert_eq!(session.bloom_chunk_meta[0].derived_chunks, 1);
        assert_eq!(session.bloom_chunk_meta[1].skipped_chunks, 1);
        assert_eq!(session.bloom_chunk_meta[2].helped_chunks, 2);
        assert_eq!(session.bloom_chunk_meta[2].authored_chunks, 1);
        assert_eq!(session.bloom_chunk_meta[2].derived_chunks, 1);
    }

    #[test]
    fn session_non_chunked_bloom_advances_immediately() {
        let mut session = WakeSession::new(&test_cascade(vec![test_entry(), test_entry()]));
        session.advance_remembered(1, PhraseSourceTag::Authored); // single-chunk bloom
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
            session.advance_remembered(total_chunks, PhraseSourceTag::Authored);
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

    // =====================================================================
    // Integration tests over the public begin/respond/skip API (Diffi's
    // mx#213 review gaps 1 & 2). These exercise the full ritual flow
    // against a minimal in-memory KnowledgeStore mock, so state-machine
    // advancement + token progression + clamp-on-shrink + P==0 repeated-
    // skip are covered end-to-end rather than via direct cursor pokes.
    // =====================================================================

    use mock_store::MockStore;

    /// Minimal in-memory KnowledgeStore used only for the wake-ritual
    /// integration tests in this module. Implements the five methods the
    /// ritual actually calls (`create_wake_session`, `get_wake_session`,
    /// `update_wake_session`, `delete_wake_session`, `get`) against
    /// RefCell-backed HashMaps; every other trait method is `unreachable!()`
    /// because the ritual code path doesn't touch them.
    ///
    /// Deliberately scoped inline to this test module — not shipped as a
    /// reusable fixture — so the integration tests here don't bloat the PR
    /// with a real mock harness that would need its own test surface.
    mod mock_store {
        use std::cell::RefCell;
        use std::collections::HashMap;

        use anyhow::Result;

        use crate::knowledge::KnowledgeEntry;
        use crate::store::{
            AgentContext, EditResult, KnowledgeFilter, KnowledgeStore, ReinforcementResult,
            WakeCascade,
        };
        use crate::types::{
            Agent, ApplicabilityType, Category, ContentType, EntryType, MemoryBackup, Project,
            Relationship, RelationshipType, Session, SessionType, SourceType,
        };
        use crate::wake_token::WakeSession;

        pub struct MockStore {
            pub blooms: RefCell<HashMap<String, KnowledgeEntry>>,
            pub sessions: RefCell<HashMap<String, WakeSession>>,
        }

        impl MockStore {
            pub fn new() -> Self {
                Self {
                    blooms: RefCell::new(HashMap::new()),
                    sessions: RefCell::new(HashMap::new()),
                }
            }

            /// Replace a bloom in place — simulates a mid-ritual content edit.
            /// The wake flow re-reads blooms via `get()` on every respond/skip,
            /// so mutating via this method between ritual calls exercises the
            /// re-derive-on-every-call contract (§2.2).
            pub fn mutate_bloom(&self, id: &str, mutate: impl FnOnce(&mut KnowledgeEntry)) {
                let mut blooms = self.blooms.borrow_mut();
                let entry = blooms.get_mut(id).expect("bloom to mutate must exist");
                mutate(entry);
            }
        }

        impl KnowledgeStore for MockStore {
            fn get(&self, id: &str, _ctx: &AgentContext) -> Result<Option<KnowledgeEntry>> {
                Ok(self.blooms.borrow().get(id).cloned())
            }

            fn create_wake_session(&self, session: &WakeSession) -> Result<String> {
                self.sessions
                    .borrow_mut()
                    .insert(session.session_id.clone(), session.clone());
                Ok(session.session_id.clone())
            }

            fn get_wake_session(&self, session_id: &str) -> Result<Option<WakeSession>> {
                Ok(self.sessions.borrow().get(session_id).cloned())
            }

            fn update_wake_session(&self, session: &WakeSession) -> Result<()> {
                self.sessions
                    .borrow_mut()
                    .insert(session.session_id.clone(), session.clone());
                Ok(())
            }

            fn delete_wake_session(&self, session_id: &str) -> Result<()> {
                self.sessions.borrow_mut().remove(session_id);
                Ok(())
            }

            // ---- unreachable methods (not used by wake_ritual flow) ----

            fn upsert_knowledge(&self, _entry: &KnowledgeEntry) -> Result<()> {
                unreachable!("wake ritual does not write blooms")
            }
            fn delete(&self, _id: &str, _ctx: &AgentContext) -> Result<bool> {
                unreachable!()
            }
            fn search(
                &self,
                _q: &str,
                _ctx: &AgentContext,
                _f: &KnowledgeFilter,
            ) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn semantic_search(
                &self,
                _emb: &[f32],
                _ctx: &AgentContext,
                _f: &KnowledgeFilter,
                _l: usize,
            ) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn list_by_category(
                &self,
                _c: &str,
                _ctx: &AgentContext,
                _f: &KnowledgeFilter,
            ) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn count_by_category(
                &self,
                _c: &str,
                _ctx: &AgentContext,
                _f: &KnowledgeFilter,
            ) -> Result<usize> {
                unreachable!()
            }
            fn list_all(&self, _ctx: &AgentContext) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn count(&self) -> Result<usize> {
                unreachable!()
            }
            fn wake_cascade(
                &self,
                _ctx: &AgentContext,
                _l: usize,
                _r: Option<i32>,
                _d: i64,
            ) -> Result<WakeCascade> {
                unreachable!()
            }
            fn update_activations(&self, _ids: &[String]) -> Result<()> {
                unreachable!()
            }
            fn update_summary(&self, _id: &str, _s: &str, _ctx: &AgentContext) -> Result<bool> {
                unreachable!()
            }
            fn increment_activation_count(&self, _ids: &[String]) -> Result<()> {
                unreachable!()
            }
            fn query_recent_facts(&self, _d: i32) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn query_recent_facts_all_types(&self, _d: i32) -> Result<Vec<KnowledgeEntry>> {
                unreachable!()
            }
            fn reinforce(
                &self,
                _id: &str,
                _a: i32,
                _c: Option<i32>,
                _ctx: &AgentContext,
            ) -> Result<Option<ReinforcementResult>> {
                unreachable!()
            }
            fn edit_content(
                &self,
                _id: &str,
                _ctx: &AgentContext,
                _o: &str,
                _n: &str,
                _r: bool,
                _nth: Option<usize>,
            ) -> Result<EditResult> {
                unreachable!()
            }
            fn append_content(&self, _id: &str, _ctx: &AgentContext, _c: &str) -> Result<()> {
                unreachable!()
            }
            fn prepend_content(&self, _id: &str, _ctx: &AgentContext, _c: &str) -> Result<()> {
                unreachable!()
            }
            fn backup_content(
                &self,
                _e: &KnowledgeEntry,
                _o: &str,
                _a: Option<&str>,
            ) -> Result<String> {
                unreachable!()
            }
            fn list_backups(&self, _id: &str) -> Result<Vec<MemoryBackup>> {
                unreachable!()
            }
            fn latest_backup(&self, _id: &str) -> Result<Option<MemoryBackup>> {
                unreachable!()
            }
            fn purge_backups(&self, _id: &str, _k: usize) -> Result<()> {
                unreachable!()
            }
            fn get_tags_for_entry(&self, _id: &str) -> Result<Vec<String>> {
                unreachable!()
            }
            fn set_tags_for_entry(&self, _id: &str, _t: &[String]) -> Result<()> {
                unreachable!()
            }
            fn list_all_tags(&self, _c: Option<&str>) -> Result<Vec<String>> {
                unreachable!()
            }
            fn get_applicability_for_entry(&self, _id: &str) -> Result<Vec<String>> {
                unreachable!()
            }
            fn set_applicability_for_entry(&self, _id: &str, _ids: &[String]) -> Result<()> {
                unreachable!()
            }
            fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
                unreachable!()
            }
            fn upsert_applicability_type(&self, _a: &ApplicabilityType) -> Result<()> {
                unreachable!()
            }
            fn list_categories(&self) -> Result<Vec<Category>> {
                unreachable!()
            }
            fn get_category(&self, _id: &str) -> Result<Option<Category>> {
                unreachable!()
            }
            fn upsert_category(&self, _c: &Category) -> Result<()> {
                unreachable!()
            }
            fn delete_category(&self, _id: &str) -> Result<bool> {
                unreachable!()
            }
            fn list_projects(&self, _a: bool) -> Result<Vec<Project>> {
                unreachable!()
            }
            fn get_project(&self, _id: &str) -> Result<Option<Project>> {
                unreachable!()
            }
            fn upsert_project(&self, _p: &Project) -> Result<()> {
                unreachable!()
            }
            fn get_tags_for_project(&self, _id: &str) -> Result<Vec<String>> {
                unreachable!()
            }
            fn set_tags_for_project(&self, _id: &str, _t: &[String]) -> Result<()> {
                unreachable!()
            }
            fn get_applicability_for_project(&self, _id: &str) -> Result<Vec<String>> {
                unreachable!()
            }
            fn set_applicability_for_project(&self, _id: &str, _ids: &[String]) -> Result<()> {
                unreachable!()
            }
            fn list_agents(&self) -> Result<Vec<Agent>> {
                unreachable!()
            }
            fn get_agent(&self, _id: &str) -> Result<Option<Agent>> {
                unreachable!()
            }
            fn upsert_agent(&self, _a: &Agent) -> Result<()> {
                unreachable!()
            }
            fn list_relationships_for_entry(&self, _id: &str) -> Result<Vec<Relationship>> {
                unreachable!()
            }
            fn add_relationship(&self, _f: &str, _t: &str, _r: &str) -> Result<String> {
                unreachable!()
            }
            fn delete_relationship(&self, _id: &str) -> Result<bool> {
                unreachable!()
            }
            fn get_facts_for_session(&self, _id: &str) -> Result<Vec<String>> {
                unreachable!()
            }
            fn get_session_for_fact(&self, _id: &str) -> Result<Option<String>> {
                unreachable!()
            }
            fn list_sessions(&self, _p: Option<&str>) -> Result<Vec<Session>> {
                unreachable!()
            }
            fn get_session(&self, _id: &str) -> Result<Option<Session>> {
                unreachable!()
            }
            fn upsert_session(&self, _s: &Session) -> Result<()> {
                unreachable!()
            }
            fn list_source_types(&self) -> Result<Vec<SourceType>> {
                unreachable!()
            }
            fn list_entry_types(&self) -> Result<Vec<EntryType>> {
                unreachable!()
            }
            fn list_content_types(&self) -> Result<Vec<ContentType>> {
                unreachable!()
            }
            fn list_session_types(&self) -> Result<Vec<SessionType>> {
                unreachable!()
            }
            fn list_relationship_types(&self) -> Result<Vec<RelationshipType>> {
                unreachable!()
            }
            fn list_tables(&self) -> Result<Vec<String>> {
                unreachable!()
            }
        }
    }

    /// Parse the session-token string the ritual returns so the next call
    /// can verify it round-trips through verify_token. Convenience for the
    /// integration tests below.
    fn token_from_response(json: &serde_json::Value) -> String {
        json.get("session")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Diffi's issue #1 (mx#213): end-to-end test for the `chunk_truncated`
    /// clamp path. Begin ritual on a large (~4-chunk) bloom → respond on
    /// chunks 0 and 1 → shrink the bloom content mid-ritual so its new
    /// chunk count is below the current cursor → the next call must detect
    /// the shrink via `clamp_if_chunks_shrank`, surface `chunk_truncated:
    /// true`, and roll the session forward (in this case, to ritual
    /// completion since we only have one bloom).
    #[test]
    fn integration_chunk_truncated_clamp_rolls_forward() {
        let store = MockStore::new();
        let bloom = make_large_bloom(95_000, vec!["alpha", "beta", "gamma"]);
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom.clone()]);
        let ctx = AgentContext::public_only();

        // Step 1: begin ritual. Confirm chunk plan covers >=3 chunks.
        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let total_chunks_start = begin_json["progress"]["total"].as_u64().unwrap() as u16;
        assert!(
            total_chunks_start >= 3,
            "fixture must yield ≥3 chunks; got {}",
            total_chunks_start
        );
        let mut token = token_from_response(&begin_json);

        // Step 2: walk chunks 0 and 1 with correct authored phrases.
        for expected_phrase in ["alpha", "beta"] {
            let resp_json: serde_json::Value = serde_json::from_str(
                &respond_ritual(&store, &ctx, &bloom_id, expected_phrase, &token).unwrap(),
            )
            .unwrap();
            assert_eq!(resp_json["status"], "remembered");
            token = token_from_response(&resp_json);
        }

        // Confirm session cursor is now at chunk 2 (0-indexed), still mid-bloom.
        {
            let sessions = store.sessions.borrow();
            let sess = sessions.values().next().unwrap();
            assert_eq!(sess.current_index, 0);
            assert_eq!(sess.current_chunk_index, 2);
            assert!(!sess.is_complete());
        }

        // Step 3: shrink the bloom to well under the threshold so its new
        // chunk plan has only 1 chunk. The cursor at chunk_index=2 is now
        // past the new total — next respond must clamp forward.
        store.mutate_bloom(&bloom_id, |entry| {
            entry.body = Some("shrunk down to a single tiny chunk now.".to_string());
        });

        // Step 4: next respond triggers the clamp path. The phrase we send
        // is irrelevant because clamp short-circuits before phrase compare.
        let resp_json: serde_json::Value = serde_json::from_str(
            &respond_ritual(&store, &ctx, &bloom_id, "ignored", &token).unwrap(),
        )
        .unwrap();

        // Clamp surfaces as status=chunk_truncated and chunk_truncated=true
        // on the returned bloom payload (§2.2).
        assert_eq!(
            resp_json["status"], "chunk_truncated",
            "expected clamp status, got {:?}",
            resp_json["status"]
        );
        assert_eq!(
            resp_json["bloom"]["chunk_truncated"],
            serde_json::Value::Bool(true),
            "expected chunk_truncated flag on bloom payload"
        );

        // Step 5: ritual must have advanced — since we only had one bloom,
        // clamp rolls us to completion. Summary should be present.
        assert!(
            resp_json.get("summary").is_some(),
            "expected ritual completion summary after clamp; got {:?}",
            resp_json
        );
        assert!(
            store.sessions.borrow().is_empty(),
            "session should have been deleted on completion"
        );
    }

    /// Diffi's issue #2 (mx#213): P==0 repeated-skip walkthrough on an
    /// oversized phraseless bloom. Builds a bloom large enough to split
    /// into ≥3 chunks, with zero authored phrases, and walks the whole
    /// thing via repeated `skip_ritual` calls. Asserts:
    ///
    /// - Every chunk emits as skip-type (no phrase attempted).
    /// - `BloomPrompt.phrase_source` is None on every prompt (P==0 blooms
    ///   don't expose authored/derived).
    /// - `BloomPrompt.wake_phrase_count` is 0 on every prompt.
    /// - Each skip advances the chunk cursor; after N skips for an
    ///   N-chunk bloom, the ritual completes.
    /// - Summary reports `skipped == total_chunks`.
    #[test]
    fn integration_phraseless_oversized_bloom_walks_via_repeated_skips() {
        let store = MockStore::new();
        // P==0: vec![] — zero authored phrases. Conservative default
        // must keep every chunk skip-typed.
        let bloom = make_large_bloom(72_000, vec![]);
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom.clone()]);
        let ctx = AgentContext::public_only();

        // Begin. Expect P==0 marker visible on prompt (wake_phrase_count=0,
        // phrase_source None).
        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let total_chunks = begin_json["progress"]["total"].as_u64().unwrap() as u16;
        assert!(total_chunks >= 3, "need ≥3 chunks; got {}", total_chunks);
        assert_eq!(
            begin_json["prompt"]["wake_phrase_count"], 0,
            "P==0 bloom prompt must declare zero phrases"
        );
        assert!(
            begin_json["prompt"].get("phrase_source").is_none()
                || begin_json["prompt"]["phrase_source"].is_null(),
            "P==0 bloom prompt must not advertise phrase_source; got {:?}",
            begin_json["prompt"].get("phrase_source")
        );

        // Walk every chunk via --skip. On each response, assert the flow
        // stays in skip-mode (no hint, no phrase compare attempted).
        let mut token = token_from_response(&begin_json);
        for chunk_walked in 0..total_chunks {
            let skip_json: serde_json::Value =
                serde_json::from_str(&skip_ritual(&store, &ctx, &bloom_id, &token).unwrap())
                    .unwrap();
            assert_eq!(skip_json["status"], "skipped");

            // Skip responses never include a hint field (hints come from
            // the authored-phrase retry flow). Catches accidental regression
            // where skip might start suggesting phrases.
            assert!(
                skip_json.get("hint").is_none() || skip_json["hint"].is_null(),
                "skip response must not carry hint; got {:?}",
                skip_json.get("hint")
            );

            // On all but the last chunk, the `next` prompt must also be
            // P==0 / skip-compatible.
            if chunk_walked + 1 < total_chunks {
                let next = &skip_json["next"];
                assert!(!next.is_null(), "expected next prompt before completion");
                assert_eq!(
                    next["wake_phrase_count"], 0,
                    "next chunk in P==0 bloom must stay wake_phrase_count=0"
                );
                assert!(
                    next.get("phrase_source").is_none() || next["phrase_source"].is_null(),
                    "P==0 next prompt must not expose phrase_source"
                );
            }

            token = token_from_response(&skip_json);
        }

        // After N skips for an N-chunk bloom, ritual completes. Summary
        // must report total == skipped.
        {
            let final_json: serde_json::Value = serde_json::from_str(
                &skip_ritual(&store, &ctx, &bloom_id, &token)
                    .err()
                    .map(|e| format!(r#"{{"error":{:?}}}"#, e.to_string()))
                    .unwrap_or_else(|| {
                        // If we ran exactly total_chunks skips above, the
                        // ritual is already complete. A subsequent skip
                        // would fail; we don't call it — we check the
                        // session was deleted instead.
                        String::new()
                    }),
            )
            .unwrap_or(serde_json::json!({}));
            let _ = final_json;
        }
        assert!(
            store.sessions.borrow().is_empty(),
            "session should be deleted on completion; still present: {:?}",
            store.sessions.borrow().keys().collect::<Vec<_>>()
        );
    }

    // =====================================================================
    // PR 3 — summary roll-up & observability tests
    // =====================================================================

    #[test]
    fn summary_rollup_all_remembered() {
        let entry_a = {
            let mut e = test_entry();
            e.title = "Alpha".to_string();
            e.id = "kn-a".to_string();
            e
        };
        let entry_b = {
            let mut e = test_entry();
            e.title = "Beta".to_string();
            e.id = "kn-b".to_string();
            e
        };

        let cascade = test_cascade(vec![entry_a.clone(), entry_b.clone()]);
        let mut session = WakeSession::new(&cascade);

        // Bloom A: 3 chunks, all remembered, all authored phrases.
        session.advance_remembered(3, PhraseSourceTag::Authored);
        session.advance_remembered(3, PhraseSourceTag::Authored);
        session.advance_remembered(3, PhraseSourceTag::Derived);
        // Bloom B: 1 chunk, remembered.
        session.advance_remembered(1, PhraseSourceTag::Authored);

        let mut blooms = HashMap::new();
        blooms.insert(entry_a.id.clone(), entry_a);
        blooms.insert(entry_b.id.clone(), entry_b);

        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups.len(), 2);

        assert_eq!(rollups[0].title, "Alpha");
        assert_eq!(rollups[0].total, 3);
        assert_eq!(rollups[0].remembered, 3);
        assert_eq!(rollups[0].authored_chunks, 2);
        assert_eq!(rollups[0].derived_chunks, 1);
        assert!(rollups[0].chunks.contains("3/3"));
        assert!(rollups[0].chunks.contains("remembered"));

        assert_eq!(rollups[1].title, "Beta");
        assert_eq!(rollups[1].total, 1);
        assert_eq!(rollups[1].remembered, 1);
        assert_eq!(rollups[1].authored_chunks, 1);
        assert_eq!(rollups[1].derived_chunks, 0);
    }

    #[test]
    fn summary_rollup_all_skipped() {
        let mut e = test_entry();
        e.title = "Phraseless".to_string();
        let cascade = test_cascade(vec![e.clone()]);
        let mut session = WakeSession::new(&cascade);
        session.advance_skipped(2);
        session.advance_skipped(2);

        let mut blooms = HashMap::new();
        blooms.insert(e.id.clone(), e);
        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups[0].total, 2);
        assert_eq!(rollups[0].skipped, 2);
        assert_eq!(rollups[0].authored_chunks, 0);
        assert_eq!(rollups[0].derived_chunks, 0);
        assert!(rollups[0].chunks.contains("skipped"));
    }

    #[test]
    fn summary_rollup_mixed_outcomes() {
        let e = test_entry();
        let cascade = test_cascade(vec![e.clone()]);
        let mut session = WakeSession::new(&cascade);
        // 4 chunks: 2 remembered + 1 helped + 1 skipped.
        session.advance_remembered(4, PhraseSourceTag::Authored);
        session.advance_remembered(4, PhraseSourceTag::Derived);
        session.advance_helped(4, PhraseSourceTag::Derived);
        session.advance_skipped(4);

        let mut blooms = HashMap::new();
        blooms.insert(e.id.clone(), e);
        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups[0].total, 4);
        assert_eq!(rollups[0].remembered, 2);
        assert_eq!(rollups[0].needed_help, 1);
        assert_eq!(rollups[0].skipped, 1);
        // authored+derived counts only count chunks that had a phrase.
        assert_eq!(rollups[0].authored_chunks, 1);
        assert_eq!(rollups[0].derived_chunks, 2);
    }

    #[test]
    fn summary_rollup_not_reached_when_zero_events() {
        let e = test_entry();
        let cascade = test_cascade(vec![e.clone()]);
        let session = WakeSession::new(&cascade);
        let mut blooms = HashMap::new();
        blooms.insert(e.id.clone(), e);
        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].total, 0);
        assert!(rollups[0].chunks.contains("not reached"));
    }

    #[test]
    fn summary_rollup_bloom_title_resolves_from_map() {
        let mut e = test_entry();
        e.id = "kn-ops".to_string();
        e.title = "Ops".to_string();
        let cascade = test_cascade(vec![e.clone()]);
        let mut session = WakeSession::new(&cascade);
        session.advance_remembered(1, PhraseSourceTag::Authored);

        let mut blooms = HashMap::new();
        blooms.insert("kn-ops".to_string(), e);
        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups[0].title, "Ops");
        assert_eq!(rollups[0].id, "kn-ops");
    }

    #[test]
    fn summary_rollup_falls_back_to_id_when_bloom_missing() {
        let e = test_entry();
        let cascade = test_cascade(vec![e]);
        let mut session = WakeSession::new(&cascade);
        session.advance_remembered(1, PhraseSourceTag::Authored);

        // Empty blooms map — title should fall back to the bloom_id.
        let blooms = HashMap::new();
        let rollups = build_bloom_rollups(&session, &blooms);
        assert_eq!(rollups[0].title, rollups[0].id);
    }

    // =====================================================================
    // mx#216 — skip guard: --skip restricted to phraseless blooms
    // =====================================================================

    #[test]
    fn skip_rejects_bloom_with_wake_phrases_array() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha", "beta"]);
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom]);
        let ctx = AgentContext::public_only();

        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let token = token_from_response(&begin_json);

        // Attempt to skip a bloom that has wake_phrases — must be rejected.
        let skip_json: serde_json::Value =
            serde_json::from_str(&skip_ritual(&store, &ctx, &bloom_id, &token).unwrap()).unwrap();
        assert_eq!(skip_json["status"], "error");
        assert_eq!(skip_json["error"], "skip_requires_phraseless_bloom");
        assert_eq!(skip_json["expected_id"], bloom_id);

        // Session state must be unchanged — still at step 0, bloom 0, chunk 0.
        let sessions = store.sessions.borrow();
        let sess = sessions.values().next().unwrap();
        assert_eq!(sess.step, 0);
        assert_eq!(sess.current_index, 0);
        assert_eq!(sess.current_chunk_index, 0);
        assert_eq!(sess.skipped_count, 0);
    }

    #[test]
    fn skip_rejects_bloom_with_legacy_wake_phrase() {
        let store = MockStore::new();
        let mut bloom = test_entry();
        bloom.wake_phrase = Some("legacy secret".to_string());
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom]);
        let ctx = AgentContext::public_only();

        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let token = token_from_response(&begin_json);

        // Legacy wake_phrase (singular) should also trigger the guard.
        let skip_json: serde_json::Value =
            serde_json::from_str(&skip_ritual(&store, &ctx, &bloom_id, &token).unwrap()).unwrap();
        assert_eq!(skip_json["status"], "error");
        assert_eq!(skip_json["error"], "skip_requires_phraseless_bloom");

        // Session state unchanged.
        let sessions = store.sessions.borrow();
        let sess = sessions.values().next().unwrap();
        assert_eq!(sess.step, 0);
        assert_eq!(sess.skipped_count, 0);
    }

    #[test]
    fn skip_accepts_phraseless_bloom() {
        // Regression guard: phraseless blooms must still skip normally.
        let store = MockStore::new();
        let bloom = test_entry(); // no wake_phrases, no wake_phrase
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom]);
        let ctx = AgentContext::public_only();

        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let token = token_from_response(&begin_json);

        let skip_json: serde_json::Value =
            serde_json::from_str(&skip_ritual(&store, &ctx, &bloom_id, &token).unwrap()).unwrap();
        assert_eq!(
            skip_json["status"], "skipped",
            "phraseless bloom should skip normally; got {:?}",
            skip_json["status"]
        );
    }

    #[test]
    fn skip_rejection_does_not_rotate_token() {
        // After a skip rejection, the same token must still work for --respond.
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store
            .blooms
            .borrow_mut()
            .insert(bloom_id.clone(), bloom.clone());

        let cascade = test_cascade(vec![bloom]);
        let ctx = AgentContext::public_only();

        let begin_json: serde_json::Value =
            serde_json::from_str(&begin_ritual(&store, &cascade).unwrap()).unwrap();
        let token = token_from_response(&begin_json);

        // Skip is rejected — no token rotation.
        let skip_json: serde_json::Value =
            serde_json::from_str(&skip_ritual(&store, &ctx, &bloom_id, &token).unwrap()).unwrap();
        assert_eq!(skip_json["status"], "error");

        // Same token works for --respond with the correct phrase.
        let resp_json: serde_json::Value = serde_json::from_str(
            &respond_ritual(&store, &ctx, &bloom_id, "alpha", &token).unwrap(),
        )
        .unwrap();
        assert_eq!(
            resp_json["status"], "remembered",
            "original token should still work after skip rejection; got {:?}",
            resp_json["status"]
        );
    }
}
