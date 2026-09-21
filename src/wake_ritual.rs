use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::engage::{MatchResult, fuzzy_match};
use crate::knowledge::KnowledgeEntry;
use crate::store::{AgentContext, KnowledgeStore, WakeCascade};
use crate::wake_chunk::{
    ChunkPlan, PhraseMatch, chunk_threshold, compare_phrase, compute_chunks, extract_auto_phrase,
    extract_salient_phrase,
};
use crate::wake_guess::{Bucket, MatchKind, WakeGuessRow, content_hash};
use crate::wake_token::*;

/// The phrases a chunk's guess is matched against, and where they came from.
///
/// For an `Authored` chunk this is every authored phrase on the bloom, not
/// just the one at the chunk's index: a single-chunk bloom with three phrases
/// used to be matched against phrase 0 only, which made phrases 1 and 2
/// unreachable (#450). For `Derived` and `Auto` it is the one generated phrase.
struct ChunkPhrases {
    phrases: Vec<String>,
    source: PhraseSource,
}

/// Pick the phrase set for a specific chunk of a bloom.
///
/// The source is decided by position: `authored` while the chunk index is
/// below the bloom's authored-phrase count, `derived` beyond it, and `auto`
/// for a bloom with no authored phrases at all. Since mx#218 every chunk
/// resolves to a non-empty phrase list.
fn phrases_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u16,
    chunk_total: u16,
    chunk_content: &str,
) -> ChunkPhrases {
    let authored = authored_phrases(entry);
    if authored.is_empty() {
        return ChunkPhrases {
            phrases: vec![extract_auto_phrase(chunk_content, &entry.title)],
            source: PhraseSource::Auto,
        };
    }
    if (chunk_idx as usize) < authored.len() {
        return ChunkPhrases {
            phrases: authored,
            source: PhraseSource::Authored,
        };
    }
    ChunkPhrases {
        phrases: vec![extract_salient_phrase(
            chunk_content,
            chunk_idx,
            chunk_total,
        )],
        source: PhraseSource::Derived,
    }
}

/// Compare a guess against every phrase in the set and keep the best result.
/// `exact` beats `close`; among equals the earliest phrase wins.
fn best_match(guess: &str, phrases: &[String]) -> (MatchKind, Option<usize>) {
    let mut best: Option<usize> = None;
    for (idx, phrase) in phrases.iter().enumerate() {
        let kind = match compare_phrase(guess, phrase) {
            PhraseMatch::Exact => MatchKind::Exact,
            PhraseMatch::Tolerant => MatchKind::Close,
            PhraseMatch::Mismatch => match fuzzy_match(guess, phrase) {
                MatchResult::Exact => MatchKind::Exact,
                MatchResult::Close => MatchKind::Close,
                MatchResult::Partial | MatchResult::Wrong => MatchKind::None,
            },
        };
        match kind {
            MatchKind::Exact => return (MatchKind::Exact, Some(idx)),
            MatchKind::Close if best.is_none() => best = Some(idx),
            _ => {}
        }
    }
    match best {
        Some(idx) => (MatchKind::Close, Some(idx)),
        None => (MatchKind::None, None),
    }
}

/// Compute the sum of chunk counts across all blooms in the session, using
/// the current in-memory content.
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

/// The title as the responder sees it: decorated with `(Part N/M)` for a
/// chunked bloom, plain otherwise.
fn title_for_chunk(entry: &KnowledgeEntry, chunk_idx: u16, plan: &ChunkPlan) -> String {
    if plan.total > 1 {
        format!("{} (Part {}/{})", entry.title, chunk_idx + 1, plan.total)
    } else {
        entry.title.clone()
    }
}

fn chunk_ref(chunk_idx: u16, plan: &ChunkPlan) -> Option<ChunkRef> {
    if plan.total > 1 {
        Some(ChunkRef {
            index: chunk_idx + 1,
            total: plan.total,
            oversized: if plan.is_oversized(chunk_idx) {
                Some(true)
            } else {
                None
            },
        })
    } else {
        None
    }
}

/// The text of the chunk at `chunk_idx` under `plan`.
fn chunk_text<'a>(plan: &ChunkPlan, content: &'a str, chunk_idx: u16) -> &'a str {
    if plan.total > 1 {
        plan.chunk(content, chunk_idx)
    } else {
        content
    }
}

/// Build the prompt for one chunk: the title, and nothing that would answer it.
fn build_prompt_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u16,
    plan: &ChunkPlan,
    content: &str,
) -> BloomPrompt {
    let resolved = phrases_for_chunk(
        entry,
        chunk_idx,
        plan.total,
        chunk_text(plan, content, chunk_idx),
    );
    BloomPrompt {
        id: entry.id.clone(),
        title: title_for_chunk(entry, chunk_idx, plan),
        phrase_source: resolved.source.as_str().to_string(),
        chunk: chunk_ref(chunk_idx, plan),
    }
}

/// Build the full reveal for one chunk. `content` is the *chunk's* content,
/// not the whole bloom.
fn build_full_for_chunk(
    entry: &KnowledgeEntry,
    chunk_idx: u16,
    plan: &ChunkPlan,
    content: &str,
    phrases: Vec<String>,
    source: PhraseSource,
) -> BloomFull {
    BloomFull {
        id: entry.id.clone(),
        title: title_for_chunk(entry, chunk_idx, plan),
        phrases,
        phrase_source: source.as_str().to_string(),
        content: chunk_text(plan, content, chunk_idx).to_string(),
        chunk: chunk_ref(chunk_idx, plan),
    }
}

/// The longest guess the log accepts, in CHARACTERS.
///
/// A guess is model output written verbatim into the database, and nothing
/// else on the path bounds it. Characters rather than bytes so the same guess
/// is judged the same way whatever it is written in, and a refusal rather than
/// a trim so a caller is never told its guess was logged when only part of it
/// was. Nothing on this path slices by byte index.
const MAX_GUESS_CHARS: usize = 2_000;

/// Reject a guess that is not usable data, before anything is written or
/// advanced, so the caller can simply guess again.
fn validate_guess(guess: &str) -> Result<()> {
    let chars = guess.chars().count();
    if chars > MAX_GUESS_CHARS {
        bail!(
            "Guess is {} characters; the limit is {}. Send a shorter guess.",
            chars,
            MAX_GUESS_CHARS
        );
    }
    // Matching strips every non-alphanumeric character, so a guess with none
    // is indistinguishable from an empty one — and would compare equal to any
    // phrase that also strips to nothing.
    if !guess.chars().any(|c| c.is_alphanumeric()) {
        bail!("Guess is empty. Send what the title brought to mind, then continue.");
    }
    Ok(())
}

/// Identity of the ritual run, for the session and every guess row it writes.
pub struct RitualMeta {
    pub agent: String,
    /// Wake number from `--wake`. mx does not read a counter of its own.
    pub wake: Option<i64>,
    /// Model identifier from `--model`. mx has no way to discover it.
    pub model_id: Option<String>,
}

/// Start a new wake ritual session.
pub fn begin_ritual(
    db: &dyn KnowledgeStore,
    cascade: &WakeCascade,
    meta: RitualMeta,
) -> Result<String> {
    if cascade.core.is_empty() && cascade.recent.is_empty() && cascade.bridges.is_empty() {
        if !cascade.excluded.is_empty() {
            // The entries are in the graph; a tag is keeping them out. Say so,
            // and name the way back in — otherwise the only signal is an empty
            // wake set that looks like data loss.
            let counts: Vec<String> = cascade
                .excluded
                .iter()
                .map(|(tag, n)| format!("{}: {}", tag, n))
                .collect();
            bail!(
                "No blooms to wake: every entry that qualified was excluded by tag ({}). \
                 Pass --include-excluded to wake them anyway, or remove the tag.",
                counts.join(", ")
            );
        }
        bail!("No blooms to wake");
    }

    let session = WakeSession::new(cascade, meta.agent, meta.wake, meta.model_id);

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
            bloom_current: 1,
            bloom_total: session.total_blooms(),
            buckets: None,
        },
        excluded: cascade.excluded.clone(),
    };

    Ok(serde_json::to_string(&response)?)
}

/// Judge one guess and show the bloom.
///
/// The guess is matched against the chunk's phrases, the outcome is written to
/// the guess log, and only then does the session advance. A failed log write
/// fails the call and leaves the session where it was — the guess is the data
/// the ritual exists to collect, so it is not best-effort.
pub fn respond_ritual(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_id: &str,
    guess: &str,
    token_str: &str,
) -> Result<String> {
    let (session_id, token_step) =
        verify_token(token_str).map_err(|e| anyhow::anyhow!("Token verification failed: {}", e))?;

    let mut session = db.get_wake_session(&session_id)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Session not found: {}. Run `mx memory wake --begin` to start a ritual.",
            session_id
        )
    })?;

    // `--begin` always stamps the calling agent, so a session with no agent is
    // one an older binary wrote. Walking it would file every guess under an
    // empty agent, unreachable from a log that is keyed by agent.
    if session.agent.is_empty() {
        bail!(
            "This session was created by an older version of mx and cannot be continued. \
             Run `mx memory wake --begin` to start a new ritual."
        );
    }

    // The token authorises the session, not the holder. A caller that names a
    // different agent would otherwise drive someone else's ritual, and every
    // row it wrote would be stamped with the owner's agent. A caller that
    // names no agent cannot be impersonating one; the CLI always names one.
    if let Some(caller) = ctx.agent_id.as_deref()
        && caller != session.agent
    {
        bail!(
            "This ritual was begun by another agent. Run `mx memory wake --begin` \
             to start your own."
        );
    }

    // Anti-replay: token step must match server-side state. Every respond
    // advances the step, so a second respond on the same step lands here.
    if session.step != token_step {
        bail!(
            "Token out of sync: token step {} but session at step {}",
            token_step,
            session.step
        );
    }

    validate_guess(guess)?;

    // Entries can be deleted while a ritual is open. Fetch what is still
    // there rather than failing on the first id that is gone: an entry the
    // ritual already walked past must not brick the rest of it.
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

    // The entry on the table is the one that vanished. Step over it: there is
    // nothing to show and nothing to judge.
    let Some(bloom) = all_blooms.get(&expected_id) else {
        session.advance_unjudged();
        return Ok(serde_json::to_string(&unjudged_response(
            db,
            &mut session,
            &session_id,
            &all_blooms,
            "bloom_missing",
            None,
        )?)?);
    };

    let content = bloom_content(bloom);
    let plan = compute_chunks(&content, chunk_threshold());

    // If the bloom shrank past our chunk cursor, advance to the next bloom.
    // No guess was judged, so no row is written — but the step still ticks, so
    // the token the caller just spent stops verifying.
    if session.clamp_if_chunks_shrank(plan.total) {
        let resolved = phrases_for_chunk(bloom, 0, plan.total, chunk_text(&plan, &content, 0));
        let shown =
            build_full_for_chunk(bloom, 0, &plan, &content, resolved.phrases, resolved.source);
        return Ok(serde_json::to_string(&unjudged_response(
            db,
            &mut session,
            &session_id,
            &all_blooms,
            "chunk_truncated",
            Some(shown),
        )?)?);
    }

    let chunk_idx = session.current_chunk_index;
    let chunk_content = chunk_text(&plan, &content, chunk_idx);
    let resolved = phrases_for_chunk(bloom, chunk_idx, plan.total, chunk_content);

    let (match_kind, match_index) = best_match(guess, &resolved.phrases);
    let bucket = match match_kind {
        MatchKind::None => Bucket::Revealed,
        _ => Bucket::Unhinted,
    };

    db.insert_wake_guess(&WakeGuessRow {
        agent: session.agent.clone(),
        wake: session.wake,
        session_id: session_id.clone(),
        bloom_id: expected_id.clone(),
        chunk_index: chunk_idx,
        chunk_total: plan.total,
        position: session.step,
        bloom_position: session.current_index + 1,
        bloom_total: session.total_blooms(),
        title_shown: title_for_chunk(bloom, chunk_idx, &plan),
        guess: guess.to_string(),
        model_id: session.model_id.clone(),
        phrase_source: resolved.source.as_str().to_string(),
        phrases: resolved.phrases.clone(),
        match_kind: match_kind.as_str().to_string(),
        match_index,
        bucket: bucket.as_str().to_string(),
        content_hash: content_hash(chunk_content),
    })?;

    let shown = build_full_for_chunk(
        bloom,
        chunk_idx,
        &plan,
        &content,
        resolved.phrases,
        resolved.source,
    );

    session.advance(plan.total, bucket, resolved.source);
    skip_missing_blooms(&mut session, &all_blooms);

    let (next, progress, summary) = get_next_and_progress(&session, &all_blooms)?;

    if session.is_complete() {
        db.delete_wake_session(&session_id)?;
    } else {
        db.update_wake_session(&session)?;
    }

    let response = WakeRespondResponse {
        status: "shown".to_string(),
        bucket: Some(bucket.as_str().to_string()),
        guess: Some(guess.to_string()),
        match_info: Some(MatchInfo {
            kind: match_kind.as_str().to_string(),
            phrase_index: match_index,
        }),
        bloom: Some(shown),
        session: create_token(&session_id, session.step),
        next,
        progress: Some(progress),
        summary,
    };

    Ok(serde_json::to_string(&response)?)
}

/// Finish a step where no guess was judged: persist the already-advanced
/// session, step over any blooms that have since been deleted, and build the
/// response. Shared by the truncated-chunk and deleted-entry paths, which
/// differ only in their status and in whether there is anything to show.
fn unjudged_response(
    db: &dyn KnowledgeStore,
    session: &mut WakeSession,
    session_id: &str,
    all_blooms: &HashMap<String, KnowledgeEntry>,
    status: &str,
    shown: Option<BloomFull>,
) -> Result<WakeRespondResponse> {
    skip_missing_blooms(session, all_blooms);

    let (next, progress, summary) = get_next_and_progress(session, all_blooms)?;
    if session.is_complete() {
        db.delete_wake_session(session_id)?;
    } else {
        db.update_wake_session(session)?;
    }

    Ok(WakeRespondResponse {
        status: status.to_string(),
        bucket: None,
        guess: None,
        match_info: None,
        bloom: shown,
        session: create_token(session_id, session.step),
        next,
        progress: Some(progress),
        summary,
    })
}

/// Step the cursor over blooms that are no longer in the database, so the
/// next prompt names an entry that still exists. Each one costs a step, the
/// same as any other advance.
fn skip_missing_blooms(session: &mut WakeSession, all_blooms: &HashMap<String, KnowledgeEntry>) {
    while !session.is_complete() && session.current_bloom_is_missing(all_blooms) {
        session.advance_unjudged();
    }
}

/// Fetch the blooms of a session that still exist.
///
/// Deliberately tolerant: a bloom the ritual already walked past can be
/// deleted from another shell, and re-fetching every id on every respond used
/// to hard-fail on the first one missing — bricking a session that could
/// neither be finished nor cleared. Callers handle an absent current bloom.
fn fetch_blooms_by_ids(
    db: &dyn KnowledgeStore,
    ctx: &AgentContext,
    bloom_ids: &[String],
) -> Result<HashMap<String, KnowledgeEntry>> {
    let mut map = HashMap::new();

    for id in bloom_ids {
        if let Some(entry) = db.get(id, ctx)? {
            map.insert(id.clone(), entry);
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
    // Re-compute total chunks for progress (cheap; keeps the total fresh for
    // mid-ritual edits).
    let total_chunks = total_chunks_across_cascade(session, all_blooms).max(1);

    // `current` is the chunk being worked on: one past the ones already
    // walked. When the ritual is complete there is no such chunk, so it is the
    // count walked — otherwise the last response of every ritual would report
    // a position past the end, next to a summary that says otherwise. Clamped
    // because a step where no guess was judged still ticks, and the recomputed
    // total need not have a chunk for it.
    let walked = session.step as usize;
    let display_current = if session.is_complete() {
        walked
    } else {
        walked + 1
    }
    .min(total_chunks);

    let bloom_current = (session.current_index + 1).min(session.total_blooms());
    let buckets = session.bucket_totals();

    let progress = Progress {
        current: display_current,
        total: total_chunks,
        bloom_current,
        bloom_total: session.total_blooms(),
        buckets: Some(buckets.totals()),
    };

    if session.is_complete() {
        let summary = Summary {
            chunks: session.step as usize,
            blooms: session.total_blooms(),
            buckets,
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

        Ok((
            Some(build_prompt_for_chunk(
                next_bloom,
                session.current_chunk_index,
                &next_plan,
                &next_content,
            )),
            progress,
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wake_chunk::PhraseMatch;

    // =====================================================================
    // Fixtures. Every bloom here is invented for the test.
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

    fn test_cascade(entries: Vec<KnowledgeEntry>) -> WakeCascade {
        WakeCascade {
            core: entries,
            ..Default::default()
        }
    }

    fn meta() -> RitualMeta {
        RitualMeta {
            agent: "test-agent".to_string(),
            wake: Some(7),
            model_id: Some("test-model".to_string()),
        }
    }

    /// A bloom big enough to split into several chunks, with H2 sections so
    /// the chunker has semantic break points.
    fn make_large_bloom(target_bytes: usize, phrases: Vec<&str>) -> KnowledgeEntry {
        let mut body = String::new();
        let mut section = 0;
        while body.len() < target_bytes {
            section += 1;
            body.push_str(&format!(
                "\n## Section {section}\n\n\
                 This is section {section} of the test bloom. It contains \
                 enough text that multiple sections will cross the chunking \
                 threshold. The wake ritual should walk each chunk in turn.\n\n\
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

    /// A guess that matches nothing in any fixture here.
    const MISS: &str = "zzz quibbling flugelhorn marmalade";

    fn token_from_response(json: &serde_json::Value) -> String {
        json.get("session")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn begin(store: &MockStore, cascade: &WakeCascade) -> serde_json::Value {
        serde_json::from_str(&begin_ritual(store, cascade, meta()).unwrap()).unwrap()
    }

    fn respond(store: &MockStore, bloom_id: &str, guess: &str, token: &str) -> serde_json::Value {
        let ctx = AgentContext::public_only();
        serde_json::from_str(&respond_ritual(store, &ctx, bloom_id, guess, token).unwrap()).unwrap()
    }

    // =====================================================================
    // phrases_for_chunk — which phrases a chunk is matched against
    // =====================================================================

    #[test]
    fn authored_chunk_is_matched_against_every_authored_phrase() {
        let e = entry_with_phrases(vec!["alpha", "beta", "gamma"]);
        let resolved = phrases_for_chunk(&e, 0, 5, "chunk 0 content");
        assert_eq!(resolved.source, PhraseSource::Authored);
        assert_eq!(resolved.phrases, vec!["alpha", "beta", "gamma"]);

        // Same set at a later authored index — the index picks the SOURCE,
        // not a single phrase.
        let resolved = phrases_for_chunk(&e, 2, 5, "chunk 2 content");
        assert_eq!(resolved.source, PhraseSource::Authored);
        assert_eq!(resolved.phrases, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn chunk_beyond_authored_count_derives_one_phrase() {
        let e = entry_with_phrases(vec!["alpha"]);
        let resolved = phrases_for_chunk(&e, 3, 5, "\n## Derived heading here\n\nbody text");
        assert_eq!(resolved.source, PhraseSource::Derived);
        assert_eq!(resolved.phrases, vec!["Derived heading here"]);
    }

    #[test]
    fn phraseless_bloom_gets_one_auto_phrase() {
        let e = entry_with_phrases(vec![]);
        let resolved = phrases_for_chunk(&e, 2, 3, "## A heading\n\nbody");
        assert_eq!(resolved.source, PhraseSource::Auto);
        assert_eq!(resolved.phrases, vec!["A heading"]);
    }

    #[test]
    fn legacy_single_wake_phrase_is_treated_as_authored() {
        let mut e = test_entry();
        e.wake_phrase = Some("legacy phrase".to_string());
        let resolved = phrases_for_chunk(&e, 0, 1, "chunk");
        assert_eq!(resolved.source, PhraseSource::Authored);
        assert_eq!(resolved.phrases, vec!["legacy phrase"]);
    }

    // =====================================================================
    // best_match — exact beats close, earliest phrase wins
    // =====================================================================

    #[test]
    fn best_match_reports_the_index_of_the_matching_phrase() {
        let phrases = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(best_match("gamma", &phrases), (MatchKind::Exact, Some(2)));
        assert_eq!(best_match("beta", &phrases), (MatchKind::Exact, Some(1)));
        assert_eq!(best_match(MISS, &phrases), (MatchKind::None, None));
    }

    #[test]
    fn best_match_prefers_an_exact_hit_over_an_earlier_close_one() {
        let phrases = vec!["Alpha".to_string(), "beta".to_string()];
        // "alpha" is a case-tolerant (close) hit on phrase 0; "beta" would be
        // exact. Ask for the exact one and confirm it is not shadowed.
        assert_eq!(compare_phrase("alpha", &phrases[0]), PhraseMatch::Tolerant);
        assert_eq!(best_match("beta", &phrases), (MatchKind::Exact, Some(1)));
        assert_eq!(best_match("alpha", &phrases), (MatchKind::Close, Some(0)));
    }

    // =====================================================================
    // The one-guess flow
    // =====================================================================

    #[test]
    fn a_matching_guess_is_unhinted_and_shows_the_bloom() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let token = token_from_response(&begin_json);
        let resp = respond(&store, &bloom_id, "alpha", &token);

        assert_eq!(resp["status"], "shown");
        assert_eq!(resp["bucket"], "unhinted");
        assert_eq!(resp["guess"], "alpha");
        assert_eq!(resp["match"]["kind"], "exact");
        assert_eq!(resp["match"]["phrase_index"], 0);
        assert_eq!(resp["bloom"]["content"], "body");
        assert_eq!(resp["bloom"]["phrase_source"], "authored");
    }

    #[test]
    fn a_missed_guess_is_revealed_and_still_shows_the_bloom() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let token = token_from_response(&begin_json);
        let resp = respond(&store, &bloom_id, MISS, &token);

        assert_eq!(resp["status"], "shown");
        assert_eq!(resp["bucket"], "revealed");
        assert_eq!(resp["match"]["kind"], "none");
        assert!(resp["match"]["phrase_index"].is_null());
        // The bloom is shown on a miss too — there is no second attempt.
        assert_eq!(resp["bloom"]["content"], "body");
        assert_eq!(resp["bloom"]["phrases"][0], "alpha");
    }

    #[test]
    fn one_guess_ends_the_bloom_so_a_second_respond_is_rejected() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);
        let second = entry_with_phrases(vec!["bravo"]);
        let mut second = second;
        second.id = "kn-second".to_string();
        store.seed(&second);

        let begin_json = begin(&store, &test_cascade(vec![bloom, second]));
        let token = token_from_response(&begin_json);

        // First guess is judged and the session advances past the bloom.
        let resp = respond(&store, &bloom_id, MISS, &token);
        assert_eq!(resp["status"], "shown");

        // Re-using the begin token replays a consumed step.
        let ctx = AgentContext::public_only();
        let err = respond_ritual(&store, &ctx, &bloom_id, "alpha", &token).unwrap_err();
        assert!(
            err.to_string().contains("Token out of sync"),
            "expected a replay rejection, got: {err}"
        );
        assert_eq!(
            store.guesses.borrow().len(),
            1,
            "no row for a rejected call"
        );
    }

    #[test]
    fn the_response_carries_no_hint_ladder_and_no_retired_vocabulary() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let raw = respond_ritual(
            &store,
            &AgentContext::public_only(),
            &bloom_id,
            MISS,
            &token_from_response(&begin_json),
        )
        .unwrap();

        for retired in [
            "hint",
            "attempt",
            "remembered",
            "needed_help",
            "incorrect",
            "skipped",
            "match_type",
            "derived_phrase_mismatch",
            "wake_phrase_count",
            "matched_phrase",
            "all_phrases",
            "blooms_complete",
        ] {
            let key = format!("\"{retired}\"");
            assert!(
                !raw.contains(&key),
                "respond payload still carries {key}: {raw}"
            );
        }
    }

    #[test]
    fn a_guess_matching_the_second_or_third_authored_phrase_is_unhinted() {
        // The old matcher compared chunk i against wake_phrases[i] only, so on
        // a single-chunk bloom phrases 1 and 2 could never match (#450).
        for (guess, expected_index) in [("beta", 1), ("gamma", 2)] {
            let store = MockStore::new();
            let bloom = entry_with_phrases(vec!["alpha", "beta", "gamma"]);
            let bloom_id = bloom.id.clone();
            store.seed(&bloom);

            let begin_json = begin(&store, &test_cascade(vec![bloom]));
            let resp = respond(&store, &bloom_id, guess, &token_from_response(&begin_json));

            assert_eq!(resp["bucket"], "unhinted", "guess {guess:?}");
            assert_eq!(resp["match"]["kind"], "exact", "guess {guess:?}");
            assert_eq!(resp["match"]["phrase_index"], expected_index);
            assert_eq!(store.guesses.borrow()[0].match_index, Some(expected_index));
        }
    }

    #[test]
    fn an_authored_phrase_matches_case_insensitively() {
        // Authored phrases used to need a case-sensitive exact match.
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["The Long Way Round"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let resp = respond(
            &store,
            &bloom_id,
            "the long way round.",
            &token_from_response(&begin_json),
        );
        assert_eq!(resp["bucket"], "unhinted");
        assert_eq!(resp["match"]["kind"], "close");
        assert_eq!(resp["match"]["phrase_index"], 0);
    }

    // =====================================================================
    // The guess log
    // =====================================================================

    #[test]
    fn each_respond_writes_exactly_one_row_with_both_positions() {
        let store = MockStore::new();
        let mut first = entry_with_phrases(vec!["alpha"]);
        first.id = "kn-first".to_string();
        first.title = "First".to_string();
        let mut second = entry_with_phrases(vec!["bravo"]);
        second.id = "kn-second".to_string();
        second.title = "Second".to_string();
        store.seed(&first);
        store.seed(&second);

        let begin_json = begin(&store, &test_cascade(vec![first, second]));
        let mut token = token_from_response(&begin_json);
        for (bloom_id, guess) in [("kn-first", "alpha"), ("kn-second", MISS)] {
            let resp = respond(&store, bloom_id, guess, &token);
            token = token_from_response(&resp);
        }

        let rows = store.guesses.borrow();
        assert_eq!(rows.len(), 2, "one row per respond");

        assert_eq!(rows[0].bloom_id, "kn-first");
        assert_eq!(rows[0].position, 0);
        assert_eq!(rows[0].bloom_position, 1);
        assert_eq!(rows[0].bloom_total, 2);
        assert_eq!(rows[0].chunk_index, 0);
        assert_eq!(rows[0].chunk_total, 1);
        assert_eq!(rows[0].title_shown, "First");
        assert_eq!(rows[0].bucket, "unhinted");
        assert_eq!(rows[0].match_kind, "exact");
        assert_eq!(rows[0].phrases, vec!["alpha"]);
        assert_eq!(
            rows[0].content_hash,
            crate::wake_guess::content_hash("body")
        );

        assert_eq!(rows[1].bloom_id, "kn-second");
        assert_eq!(rows[1].position, 1);
        assert_eq!(rows[1].bloom_position, 2);
        assert_eq!(rows[1].bucket, "revealed");
        assert_eq!(rows[1].match_kind, "none");
        assert_eq!(rows[1].match_index, None);
        assert_eq!(rows[1].guess, MISS);
    }

    #[test]
    fn the_wake_and_model_flags_land_on_every_row() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        respond(
            &store,
            &bloom_id,
            "alpha",
            &token_from_response(&begin_json),
        );

        let rows = store.guesses.borrow();
        assert_eq!(rows[0].wake, Some(7));
        assert_eq!(rows[0].model_id.as_deref(), Some("test-model"));
        assert_eq!(rows[0].agent, "test-agent");
    }

    #[test]
    fn an_omitted_wake_number_logs_as_absent() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json: serde_json::Value = serde_json::from_str(
            &begin_ritual(
                &store,
                &test_cascade(vec![bloom]),
                RitualMeta {
                    agent: "test-agent".to_string(),
                    wake: None,
                    model_id: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
        respond(
            &store,
            &bloom_id,
            "alpha",
            &token_from_response(&begin_json),
        );

        let rows = store.guesses.borrow();
        assert_eq!(rows[0].wake, None);
        assert_eq!(rows[0].model_id, None);
    }

    #[test]
    fn a_chunked_bloom_advances_position_but_not_bloom_position() {
        let store = MockStore::new();
        let bloom = make_large_bloom(95_000, vec!["alpha", "beta", "gamma"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let total_chunks = begin_json["progress"]["total"].as_u64().unwrap();
        assert!(total_chunks >= 3, "fixture must chunk; got {total_chunks}");

        let mut token = token_from_response(&begin_json);
        for _ in 0..total_chunks {
            let resp = respond(&store, &bloom_id, MISS, &token);
            token = token_from_response(&resp);
        }

        let rows = store.guesses.borrow();
        assert_eq!(rows.len(), total_chunks as usize, "one row per chunk");
        for (idx, row) in rows.iter().enumerate() {
            assert_eq!(row.position, idx as u32, "position advances per chunk");
            assert_eq!(row.bloom_position, 1, "still the same bloom");
            assert_eq!(row.chunk_index, idx as u16);
            assert!(
                row.title_shown.contains(&format!("(Part {}/", idx + 1)),
                "chunked title must carry its part suffix: {}",
                row.title_shown
            );
        }
    }

    #[test]
    fn a_chunk_truncated_response_writes_no_row() {
        let store = MockStore::new();
        let bloom = make_large_bloom(95_000, vec!["alpha", "beta", "gamma"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let mut token = token_from_response(&begin_json);

        // Walk two chunks, then shrink the bloom under the cursor.
        for _ in 0..2 {
            let resp = respond(&store, &bloom_id, MISS, &token);
            token = token_from_response(&resp);
        }
        assert_eq!(store.guesses.borrow().len(), 2);

        store.mutate_bloom(&bloom_id, |entry| {
            entry.body = Some("shrunk down to a single tiny chunk now.".to_string());
        });

        let resp = respond(&store, &bloom_id, "ignored", &token);
        assert_eq!(resp["status"], "chunk_truncated");
        assert!(
            resp.get("bucket").is_none(),
            "no bucket without a judgement"
        );
        assert!(resp.get("guess").is_none());
        assert_eq!(
            store.guesses.borrow().len(),
            2,
            "a truncated chunk judges no guess, so it logs none"
        );
    }

    #[test]
    fn a_failed_row_write_fails_the_respond_and_leaves_the_session_put() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        let bloom_id = bloom.id.clone();
        store.seed(&bloom);

        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        let token = token_from_response(&begin_json);

        store.fail_guess_write.set(true);
        let err = respond_ritual(
            &store,
            &AgentContext::public_only(),
            &bloom_id,
            "alpha",
            &token,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("guess log unavailable"),
            "the write failure must surface, got: {err}"
        );

        {
            let sessions = store.sessions.borrow();
            let session = sessions.values().next().expect("session must survive");
            assert_eq!(
                session.step, 0,
                "session must not advance past a lost guess"
            );
            assert_eq!(session.current_index, 0);
            assert_eq!(session.unhinted_count, 0);
        }

        // With the log back, the same token still works.
        store.fail_guess_write.set(false);
        let resp = respond(&store, &bloom_id, "alpha", &token);
        assert_eq!(resp["status"], "shown");
        assert_eq!(store.guesses.borrow().len(), 1);
    }

    #[test]
    fn a_bloom_deleted_while_it_is_on_the_table_is_stepped_over() {
        // Poppy's brick case covers an entry already walked past. This is the
        // harder one: the entry the ritual is asking about right now.
        let store = MockStore::new();
        let mut first = entry_with_phrases(vec!["alpha"]);
        first.id = "kn-first".to_string();
        let mut second = entry_with_phrases(vec!["bravo"]);
        second.id = "kn-second".to_string();
        store.seed(&first);
        store.seed(&second);

        let begin_json = begin(&store, &test_cascade(vec![first, second]));
        let token = token_from_response(&begin_json);
        store.blooms.borrow_mut().remove("kn-first");

        let resp = respond(&store, "kn-first", "alpha", &token);
        assert_eq!(resp["status"], "bloom_missing");
        assert!(resp.get("bloom").is_none(), "there is nothing to show");
        assert!(resp.get("bucket").is_none(), "nothing was judged");
        assert!(
            store.guesses.borrow().is_empty(),
            "a vanished bloom logs no guess"
        );

        // The ritual keeps going: the next entry is served and answerable.
        assert_eq!(resp["next"]["id"], "kn-second");
        let resp = respond(&store, "kn-second", "bravo", &token_from_response(&resp));
        assert_eq!(resp["status"], "shown");
        assert_eq!(resp["bucket"], "unhinted");
        // The skipped step is counted as walked but sits in no bucket.
        assert_eq!(resp["summary"]["chunks"], 2);
        assert_eq!(resp["summary"]["buckets"]["unhinted"]["authored"], 1);
        assert_eq!(resp["summary"]["buckets"]["revealed"]["authored"], 0);
    }

    #[test]
    fn a_bloom_deleted_further_down_the_sequence_is_never_prompted_for() {
        // The `next` pointer must name an entry that still exists.
        let store = MockStore::new();
        let mut first = entry_with_phrases(vec!["alpha"]);
        first.id = "kn-first".to_string();
        let mut second = entry_with_phrases(vec!["bravo"]);
        second.id = "kn-second".to_string();
        let mut third = entry_with_phrases(vec!["charlie"]);
        third.id = "kn-third".to_string();
        store.seed(&first);
        store.seed(&second);
        store.seed(&third);

        let begin_json = begin(&store, &test_cascade(vec![first, second, third]));
        let token = token_from_response(&begin_json);
        store.blooms.borrow_mut().remove("kn-second");

        let resp = respond(&store, "kn-first", "alpha", &token);
        assert_eq!(resp["status"], "shown");
        assert_eq!(
            resp["next"]["id"], "kn-third",
            "the deleted entry must be stepped over, not prompted for: {resp}"
        );
    }

    // =====================================================================
    // Progress and summary
    // =====================================================================

    #[test]
    fn the_summary_reports_bucket_counts_split_by_phrase_source() {
        let store = MockStore::new();
        let mut authored = entry_with_phrases(vec!["alpha"]);
        authored.id = "kn-authored".to_string();
        let mut auto = test_entry(); // no phrases at all
        auto.id = "kn-auto".to_string();
        auto.body = Some("## The Discovery\n\nBody text here.".to_string());
        store.seed(&authored);
        store.seed(&auto);

        let begin_json = begin(&store, &test_cascade(vec![authored, auto]));
        assert!(
            begin_json["progress"].get("buckets").is_none(),
            "nothing is guessed at begin time"
        );

        let mut token = token_from_response(&begin_json);
        let resp = respond(&store, "kn-authored", "alpha", &token);
        assert_eq!(resp["progress"]["buckets"]["unhinted"], 1);
        assert_eq!(resp["progress"]["buckets"]["revealed"], 0);
        token = token_from_response(&resp);

        let resp = respond(&store, "kn-auto", MISS, &token);
        let summary = &resp["summary"];
        assert_eq!(summary["chunks"], 2);
        assert_eq!(summary["blooms"], 2);
        assert_eq!(summary["buckets"]["unhinted"]["authored"], 1);
        assert_eq!(summary["buckets"]["unhinted"]["derived"], 0);
        assert_eq!(summary["buckets"]["unhinted"]["auto"], 0);
        assert_eq!(summary["buckets"]["revealed"]["auto"], 1);
        assert_eq!(summary["buckets"]["revealed"]["authored"], 0);
        assert!(
            resp.get("next").is_none(),
            "the last respond has no next prompt"
        );
        assert!(
            store.sessions.borrow().is_empty(),
            "session is deleted on completion"
        );
    }

    #[test]
    fn the_begin_response_reports_excluded_entries_per_tag() {
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        store.seed(&bloom);
        let mut cascade = test_cascade(vec![bloom]);
        cascade.excluded.insert("archive".to_string(), 2);
        cascade.excluded.insert("wake-exclude".to_string(), 1);

        let begin_json = begin(&store, &cascade);
        assert_eq!(begin_json["excluded"]["archive"], 2);
        assert_eq!(begin_json["excluded"]["wake-exclude"], 1);
    }

    #[test]
    fn a_begin_with_nothing_excluded_reports_an_empty_object() {
        // The key is always present, so a consumer reads one shape either way
        // and never has to tell an absent key from a zero count.
        let store = MockStore::new();
        let bloom = entry_with_phrases(vec!["alpha"]);
        store.seed(&bloom);
        let begin_json = begin(&store, &test_cascade(vec![bloom]));
        assert_eq!(
            begin_json["excluded"],
            serde_json::json!({}),
            "excluded must be present and empty: {begin_json}"
        );
    }

    // =====================================================================
    // Minimal in-memory KnowledgeStore for the tests above. Implements the
    // methods the ritual actually calls; every other trait method is
    // `unreachable!()` because the ritual never touches them.
    // =====================================================================

    use mock_store::MockStore;

    mod mock_store {
        use std::cell::{Cell, RefCell};
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
        use crate::wake_guess::WakeGuessRow;
        use crate::wake_token::WakeSession;

        pub struct MockStore {
            pub blooms: RefCell<HashMap<String, KnowledgeEntry>>,
            pub sessions: RefCell<HashMap<String, WakeSession>>,
            pub guesses: RefCell<Vec<WakeGuessRow>>,
            /// Simulates a guess log that refuses writes.
            pub fail_guess_write: Cell<bool>,
        }

        impl MockStore {
            pub fn new() -> Self {
                Self {
                    blooms: RefCell::new(HashMap::new()),
                    sessions: RefCell::new(HashMap::new()),
                    guesses: RefCell::new(Vec::new()),
                    fail_guess_write: Cell::new(false),
                }
            }

            pub fn seed(&self, entry: &KnowledgeEntry) {
                self.blooms
                    .borrow_mut()
                    .insert(entry.id.clone(), entry.clone());
            }

            /// Replace a bloom in place — simulates a mid-ritual content edit.
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

            fn insert_wake_guess(&self, row: &WakeGuessRow) -> Result<()> {
                if self.fail_guess_write.get() {
                    anyhow::bail!("guess log unavailable");
                }
                self.guesses.borrow_mut().push(row.clone());
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
            fn semantic_search_scored(
                &self,
                _emb: &[f32],
                _ctx: &AgentContext,
                _f: &KnowledgeFilter,
                _l: usize,
            ) -> Result<Vec<(KnowledgeEntry, f32)>> {
                unreachable!()
            }
            fn semantic_search_entries_scored(
                &self,
                _emb: &[f32],
                _ctx: &AgentContext,
                _l: usize,
            ) -> Result<Vec<(KnowledgeEntry, f32)>> {
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
            fn owned_private_matching(
                &self,
                _agent: &str,
                _query: Option<&str>,
                _filter: &KnowledgeFilter,
            ) -> Result<Vec<KnowledgeEntry>> {
                // N2: unreachable here by design — this hint query (Issue #400) is
                // only ever invoked from the List/Search handlers, never from the
                // wake ritual this MockStore exercises. The hint-count behavior is
                // covered by the SurrealDatabase-backed tests in helpers.rs.
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
                _include_excluded: bool,
            ) -> Result<WakeCascade> {
                unreachable!()
            }
            fn update_activations(&self, _ids: &[String]) -> Result<()> {
                unreachable!()
            }
            fn update_summary(&self, _id: &str, _s: &str, _ctx: &AgentContext) -> Result<bool> {
                unreachable!()
            }
            fn apply_update(
                &self,
                _id: &str,
                _spec: &crate::store_update::UpdateSpec,
                _ctx: &AgentContext,
            ) -> Result<crate::store_update::UpdateOutcome> {
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
            fn get_entries_for_session(
                &self,
                _id: &str,
                _owner: Option<&str>,
                _category: &str,
                _ctx: &AgentContext,
            ) -> Result<Vec<crate::store::DedupCandidate>> {
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

            fn delete_embedding_chunks(&self, _id: &str) -> Result<()> {
                unreachable!()
            }
            fn insert_embedding_chunk(
                &self,
                _id: &str,
                _ci: usize,
                _ct: &str,
                _to: usize,
                _tc: usize,
                _emb: &[f32],
                _m: &str,
            ) -> Result<()> {
                unreachable!()
            }
            fn semantic_search_chunks(
                &self,
                _emb: &[f32],
                _l: usize,
            ) -> Result<Vec<(String, f32)>> {
                unreachable!()
            }

            fn sweep_ghost_anchors(
                &self,
                _dry_run: bool,
            ) -> Result<crate::store::GhostSweepResult> {
                unreachable!()
            }
        }
    }

    // =====================================================================
    // Adversarial cases. Each one asserts the behaviour the spec or the
    // invariant asks for; the ones that fail are defects, not test bugs.
    // Every fixture here is invented.
    // =====================================================================
    mod adversarial {
        use super::mock_store::MockStore;
        use super::*;

        /// A guess with no alphanumeric content is not data: it is the absence
        /// of the thing the ritual exists to collect. Such a guess is refused
        /// with an error, writes no row, and does not advance the session.
        ///
        /// Refusing it also closes a matching hole. `fuzzy_match` strips every
        /// non-alphanumeric character before comparing, so a blank guess and a
        /// punctuation-only phrase both normalize to the empty string and
        /// compare EQUAL — logged as `exact`, bucketed `unhinted`.
        ///
        /// Ruled by Q, Wake 462: refuse, no row, no advance.
        #[test]
        fn a_guess_that_normalizes_to_nothing_is_refused() {
            // The empty string, whitespace, and punctuation that survives a
            // trim but carries no content.
            for guess in ["", "   ", "\t\n", "\u{2014}", "...", "???"] {
                let store = MockStore::new();
                // An em-dash phrase normalizes to nothing too, which is what
                // makes the blank guess an `exact` match today.
                let bloom = entry_with_phrases(vec!["\u{2014}"]);
                let bloom_id = bloom.id.clone();
                store.seed(&bloom);

                let begin_json = begin(&store, &test_cascade(vec![bloom]));
                let out = respond_ritual(
                    &store,
                    &AgentContext::public_only(),
                    &bloom_id,
                    guess,
                    &token_from_response(&begin_json),
                );

                assert!(
                    out.is_err(),
                    "guess {guess:?} was accepted instead of refused: {out:?}"
                );
                assert!(
                    store.guesses.borrow().is_empty(),
                    "guess {guess:?} reached the log"
                );

                let sessions = store.sessions.borrow();
                let session = sessions.values().next().expect("session must survive");
                assert_eq!(
                    session.step, 0,
                    "guess {guess:?} advanced the session it was refused from"
                );
                assert_eq!(session.current_index, 0, "guess {guess:?}");
            }
        }

        /// `fuzzy_match` counts edit distance in CHARACTERS but divides by
        /// `str::len()`, which is BYTES. For multibyte text the denominator is
        /// inflated 3-4x, so the 0.8 tolerance silently widens to accept a
        /// guess that is half wrong. Identical edit ratios must get identical
        /// verdicts regardless of how the text happens to encode.
        #[test]
        fn the_match_tolerance_does_not_widen_for_multibyte_text() {
            // Control: 5 of 10 ASCII characters differ -> not a match.
            let ascii = vec!["abcdefghij".to_string()];
            assert_eq!(
                best_match("abcdeqrstu", &ascii),
                (MatchKind::None, None),
                "precondition: half-wrong ASCII is not a match"
            );

            // Same shape in hiragana: 5 of 10 characters differ.
            let kana = vec![
                "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{304B}\u{304D}\u{304F}\u{3051}\u{3053}"
                    .to_string(),
            ];
            let half_wrong =
                "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}\u{3055}\u{3057}\u{3059}\u{305B}\u{305D}";
            assert_eq!(
                best_match(half_wrong, &kana),
                (MatchKind::None, None),
                "a half-wrong multibyte guess was accepted as a match"
            );
        }

        /// Deleting an entry that the ritual has already walked past must not
        /// break the rest of the ritual. `fetch_blooms_by_ids` re-fetches every
        /// id in the session on every respond and hard-fails on the first one
        /// that is gone, so removing an already-shown bloom bricks the session
        /// with no way to finish it and no way to clear it.
        #[test]
        fn deleting_an_already_shown_bloom_does_not_brick_the_ritual() {
            let store = MockStore::new();
            let mut first = entry_with_phrases(vec!["alpha"]);
            first.id = "kn-first".to_string();
            let mut second = entry_with_phrases(vec!["bravo"]);
            second.id = "kn-second".to_string();
            store.seed(&first);
            store.seed(&second);

            let begin_json = begin(&store, &test_cascade(vec![first, second]));
            let resp = respond(
                &store,
                "kn-first",
                "alpha",
                &token_from_response(&begin_json),
            );
            let token = token_from_response(&resp);

            // The first bloom is done with. Remove it, as a `memory delete`
            // from another shell would.
            store.blooms.borrow_mut().remove("kn-first");

            let out = respond_ritual(
                &store,
                &AgentContext::public_only(),
                "kn-second",
                "bravo",
                &token,
            );
            assert!(
                out.is_ok(),
                "a deleted, already-shown bloom killed the rest of the ritual: {:?}",
                out.err()
            );
        }

        /// A `chunk_truncated` response must retire the token it was handed.
        /// The truncation path advances the bloom cursor but never ticks
        /// `step`, so the token it returns is byte-identical to the one just
        /// spent and that spent token still verifies against the session.
        #[test]
        fn a_chunk_truncated_response_retires_the_token_it_consumed() {
            let store = MockStore::new();
            let big = make_large_bloom(95_000, vec!["alpha", "beta", "gamma"]);
            let big_id = big.id.clone();
            let mut tail = entry_with_phrases(vec!["bravo"]);
            tail.id = "kn-tail".to_string();
            tail.title = "Tail".to_string();
            store.seed(&big);
            store.seed(&tail);

            let begin_json = begin(&store, &test_cascade(vec![big, tail]));
            let mut token = token_from_response(&begin_json);
            for _ in 0..2 {
                let resp = respond(&store, &big_id, MISS, &token);
                token = token_from_response(&resp);
            }

            store.mutate_bloom(&big_id, |entry| {
                entry.body = Some("shrunk down to a single tiny chunk now.".to_string());
            });

            let spent = token.clone();
            let resp = respond(&store, &big_id, "ignored", &spent);
            assert_eq!(resp["status"], "chunk_truncated");
            assert_ne!(
                token_from_response(&resp),
                spent,
                "a truncation handed back the very token it consumed"
            );
        }

        /// The token authorises the session, not the caller, and nothing
        /// checks that the responder is the agent that began the ritual. A
        /// foreign agent can drive someone else's ritual to completion, and
        /// every row it writes is stamped with the OWNER's agent id — so the
        /// owner's guess log silently fills with guesses they never made.
        #[test]
        fn a_respond_from_another_agent_is_refused() {
            let store = MockStore::new();
            let bloom = entry_with_phrases(vec!["alpha"]);
            let bloom_id = bloom.id.clone();
            store.seed(&bloom);

            // `meta()` begins the ritual as "test-agent".
            let begin_json = begin(&store, &test_cascade(vec![bloom]));
            let out = respond_ritual(
                &store,
                &AgentContext::for_agent("some-other-agent"),
                &bloom_id,
                "alpha",
                &token_from_response(&begin_json),
            );

            assert!(out.is_err(), "another agent walked this session: {out:?}");
            assert!(
                store.guesses.borrow().is_empty(),
                "a foreign respond wrote a row stamped with the owner's agent: {:?}",
                store.guesses.borrow().first().map(|r| r.agent.clone())
            );
        }

        /// A session whose `agent` is empty cannot be one this binary began:
        /// `--begin` always stamps `MX_CURRENT_AGENT`. It is a session written
        /// by the pre-one-guess binary, whose row has no `agent` field at all
        /// and which the loader silently defaults to "". Spec 9.1 says such a
        /// session is treated as expired and the caller is told to run
        /// `--begin`. Instead it walks, and every guess row it writes carries
        /// `agent: ""` — unattributable, and invisible to every query in the
        /// log, all of which filter on the agent.
        #[test]
        fn a_session_from_an_older_binary_is_refused_rather_than_walked() {
            let store = MockStore::new();
            let bloom = entry_with_phrases(vec!["alpha"]);
            let bloom_id = bloom.id.clone();
            store.seed(&bloom);

            let mut legacy =
                WakeSession::new(&test_cascade(vec![bloom]), String::new(), None, None);
            legacy.agent = String::new();
            let session_id = legacy.session_id.clone();
            store.create_wake_session(&legacy).unwrap();
            let token = create_token(&session_id, legacy.step);

            let out = respond_ritual(
                &store,
                &AgentContext::public_only(),
                &bloom_id,
                "alpha",
                &token,
            );

            let refused = match out {
                Err(ref e) => e.to_string().contains("--begin"),
                Ok(_) => false,
            };
            assert!(
                refused,
                "an agent-less session was walked instead of refused: {out:?}"
            );
            assert!(
                store
                    .guesses
                    .borrow()
                    .iter()
                    .all(|row| !row.agent.is_empty()),
                "a guess row was written with an empty agent"
            );
        }

        /// `progress.current` is `step + 1` — "the chunk we are on now" — but
        /// after the LAST advance there is no chunk we are on, so the final
        /// response of every ritual reports a position one past the end:
        /// chunk 2 of 2 becomes `current: 3, total: 2`. This is in the payload
        /// the model reads, next to the summary.
        #[test]
        fn the_final_response_does_not_report_a_position_past_the_end() {
            let store = MockStore::new();
            let mut first = entry_with_phrases(vec!["alpha"]);
            first.id = "kn-first".to_string();
            let mut second = entry_with_phrases(vec!["bravo"]);
            second.id = "kn-second".to_string();
            store.seed(&first);
            store.seed(&second);

            let begin_json = begin(&store, &test_cascade(vec![first, second]));
            let mut token = token_from_response(&begin_json);
            let mut last = serde_json::Value::Null;
            for (id, guess) in [("kn-first", "alpha"), ("kn-second", "bravo")] {
                last = respond(&store, id, guess, &token);
                token = token_from_response(&last);
            }

            let current = last["progress"]["current"].as_u64().unwrap();
            let total = last["progress"]["total"].as_u64().unwrap();
            assert!(
                current <= total,
                "final progress reads {current} of {total}: {}",
                last["progress"]
            );
        }

        /// The guess is model output written verbatim into the database with
        /// no bound anywhere on the path. Ruled by Q, Wake 462: a guess longer
        /// than 2000 CHARACTERS is REFUSED — error, no row, no advance — not
        /// truncated. A truncated guess is not what the model guessed, and the
        /// log exists to be honest about guesses; better nothing than a row
        /// that misstates one.
        ///
        /// The limit counts characters, so both boundaries are driven in two
        /// encodings. A byte-based check would wrongly refuse the 2000-kana
        /// guess (6000 bytes) that must be accepted, and a byte-based
        /// truncation would panic on a boundary landing mid-character.
        #[test]
        fn a_guess_over_two_thousand_characters_is_refused() {
            const LIMIT: usize = 2_000;

            // A fresh session per case: an accepted guess advances one.
            fn fixture(store: &MockStore) -> (String, String) {
                let bloom = entry_with_phrases(vec!["alpha"]);
                let bloom_id = bloom.id.clone();
                store.seed(&bloom);
                let begin_json = begin(store, &test_cascade(vec![bloom]));
                (bloom_id, token_from_response(&begin_json))
            }

            // ASCII, and a 3-byte character. Both boundaries are driven in
            // both encodings, and the two directions run as separate loops so
            // a failure in one does not hide the other's cases.

            // ---- exactly at the limit: accepted, and logged WHOLE ----
            for filler in ["a", "\u{3042}"] {
                let store = MockStore::new();
                let (bloom_id, token) = fixture(&store);
                let at_limit = filler.repeat(LIMIT);
                let out = respond_ritual(
                    &store,
                    &AgentContext::public_only(),
                    &bloom_id,
                    &at_limit,
                    &token,
                );
                assert!(
                    out.is_ok(),
                    "a guess of exactly {LIMIT} characters must be accepted \
                     (filler {filler:?}): {out:?}"
                );
                let rows = store.guesses.borrow();
                let row = rows.first().expect("an accepted guess is logged");
                assert_eq!(
                    row.guess, at_limit,
                    "a guess at the limit must be logged whole, not trimmed \
                     (filler {filler:?})"
                );
            }

            // ---- one character over: refused, no row, no advance ----
            for filler in ["a", "\u{3042}"] {
                let store = MockStore::new();
                let (bloom_id, token) = fixture(&store);
                let over = filler.repeat(LIMIT + 1);
                let out = respond_ritual(
                    &store,
                    &AgentContext::public_only(),
                    &bloom_id,
                    &over,
                    &token,
                );
                assert!(
                    out.is_err(),
                    "a guess of {} characters was accepted (filler {filler:?}): {out:?}",
                    LIMIT + 1
                );
                assert!(
                    store.guesses.borrow().is_empty(),
                    "an over-long guess reached the log (filler {filler:?})"
                );

                let sessions = store.sessions.borrow();
                let session = sessions.values().next().expect("session must survive");
                assert_eq!(
                    session.step, 0,
                    "an over-long guess advanced the session it was refused from \
                     (filler {filler:?})"
                );
                assert_eq!(session.current_index, 0, "filler {filler:?}");
            }
        }

        /// Spec 9.1 reasons that "sessions live for one ritual, so at most one
        /// is affected". Nothing enforces that: `created_at` is stored and then
        /// never read, so a token minted a year ago still drives its session.
        #[test]
        #[ignore = "session lifecycle is follow-up work, not part of this change"]
        fn a_stale_session_is_expired_by_age() {
            let store = MockStore::new();
            let bloom = entry_with_phrases(vec!["alpha"]);
            let bloom_id = bloom.id.clone();
            store.seed(&bloom);

            let begin_json = begin(&store, &test_cascade(vec![bloom]));
            let token = token_from_response(&begin_json);

            // Backdate the session a year.
            {
                let mut sessions = store.sessions.borrow_mut();
                let session = sessions.values_mut().next().expect("session exists");
                session.created_at -= 365 * 24 * 60 * 60;
            }

            let out = respond_ritual(
                &store,
                &AgentContext::public_only(),
                &bloom_id,
                "alpha",
                &token,
            );
            assert!(
                out.is_err(),
                "a year-old session token still walked the ritual: {out:?}"
            );
        }

        /// A second `--begin` abandons the first session's row. Nothing deletes
        /// it and nothing sweeps by age, so every interrupted ritual leaves a
        /// permanent row behind — each one carrying a bloom id list.
        #[test]
        #[ignore = "session lifecycle is follow-up work, not part of this change"]
        fn beginning_again_does_not_orphan_the_previous_session() {
            let store = MockStore::new();
            let bloom = entry_with_phrases(vec!["alpha"]);
            store.seed(&bloom);

            begin(&store, &test_cascade(vec![bloom.clone()]));
            begin(&store, &test_cascade(vec![bloom]));

            assert_eq!(
                store.sessions.borrow().len(),
                1,
                "a second begin left the first ritual's session behind"
            );
        }
    }
}
