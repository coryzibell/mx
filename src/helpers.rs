use anyhow::{Result, bail};

use crate::cli::EntryFilter;
use crate::index::IndexConfig;
use crate::knowledge;
use crate::store;
use crate::surreal_db::SurrealDatabase;

/// Apply in-memory field presence filters to a list of entries
pub(crate) fn apply_entry_filters(
    entries: Vec<knowledge::KnowledgeEntry>,
    filter: &EntryFilter,
) -> Vec<knowledge::KnowledgeEntry> {
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|e| !filter.has_wake_phrase || e.has_any_wake_phrase())
        .filter(|e| !filter.missing_wake_phrase || !e.has_any_wake_phrase())
        .filter(|e| !filter.has_anchors || !e.anchors.is_empty())
        .filter(|e| !filter.missing_anchors || e.anchors.is_empty())
        .filter(|e| {
            !filter.has_resonance_type || e.resonance_type.as_ref().is_some_and(|s| !s.is_empty())
        })
        .filter(|e| {
            !filter.missing_resonance_type || e.resonance_type.as_ref().is_none_or(|s| s.is_empty())
        })
        .filter(|e| {
            filter
                .tags
                .as_ref()
                .is_none_or(|filter_tags| filter_tags.iter().any(|t| e.tags.contains(t)))
        })
        .collect();

    // Apply limit if specified
    if let Some(n) = filter.limit {
        entries.truncate(n);
    }

    entries
}

/// Normalize a knowledge entry ID (accept both "kn-abc" and "abc", normalize to "kn-abc")
pub(crate) fn normalize_id(id: &str) -> String {
    if id.starts_with("kn-") {
        id.to_string()
    } else {
        format!("kn-{}", id)
    }
}

/// Routing table for fact types to categories and tags
pub(crate) struct FactRouting {
    pub(crate) category: &'static str,
    pub(crate) tags: Vec<&'static str>,
}

/// Find an open thread by content match
///
/// Uses normalized content comparison to handle whitespace/formatting differences.
/// Threads without summary metadata are treated as potentially open: the close
/// handler always writes state, so absence implies never-closed (pre-convention threads).
pub(crate) fn find_open_thread_by_content(
    db: &dyn store::KnowledgeStore,
    content: &str,
    agent_id: &str,
) -> Result<String> {
    use crate::knowledge::KnowledgeEntry;

    let ctx = store::AgentContext::for_agent(agent_id);
    let filter = store::KnowledgeFilter {
        categories: Some(vec!["thread".to_string()]),
        ..Default::default()
    };

    let threads = db.list_by_category("thread", &ctx, &filter)?;
    let normalized_content = KnowledgeEntry::normalize_content(content);

    for thread in threads {
        // Check if normalized body matches and state is open (or absent — pre-convention threads)
        let is_open = match thread.get_summary_state().as_deref() {
            None => true, // Pre-convention threads lack summary metadata. Since the close
            // handler always writes state, absence implies never-closed.
            Some("open") => true,
            _ => false,
        };

        if is_open && let Some(body) = &thread.body {
            let normalized_body = KnowledgeEntry::normalize_content(body);
            if normalized_body == normalized_content {
                return Ok(thread.id);
            }
        }
    }

    bail!("No open thread found matching content: '{}'", content)
}

/// Route a fact type to its target category and tags.
/// NOTE: The category targets below (decision, insight, reference, thread) map to the default
/// seed categories in schema/surrealdb-schema.surql. Custom deployments that rename or remove
/// these seed categories must update this routing table accordingly.
pub(crate) fn route_fact_type(fact_type: &str) -> Result<FactRouting> {
    const VALID_FACT_TYPES: &[&str] = &[
        "decision",
        "insight",
        "person",
        "quote",
        "thread_opened",
        "commitment",
        "thread_closed",
    ];

    match fact_type {
        "decision" => Ok(FactRouting {
            category: "decision",
            tags: vec![],
        }),
        "insight" => Ok(FactRouting {
            category: "insight",
            tags: vec![],
        }),
        "person" => Ok(FactRouting {
            category: "reference",
            tags: vec!["person"],
        }),
        "quote" => Ok(FactRouting {
            category: "reference",
            tags: vec!["quote"],
        }),
        "thread_opened" => Ok(FactRouting {
            category: "thread",
            tags: vec!["question"],
        }),
        "commitment" => Ok(FactRouting {
            category: "thread",
            tags: vec!["commitment"],
        }),
        "thread_closed" => Ok(FactRouting {
            category: "thread",
            tags: vec![],
        }),
        unknown => {
            bail!(
                "Invalid fact type '{}'. Valid types: {}",
                unknown,
                VALID_FACT_TYPES.join(", ")
            )
        }
    }
}

/// Resolve agent context from environment and flags
pub(crate) fn resolve_agent_context(mine: bool, include_private: bool) -> store::AgentContext {
    match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => {
            if mine {
                // --mine: only show private entries owned by this agent
                store::AgentContext::for_agent(agent)
            } else if include_private {
                // --include-private: show public + private entries owned by this agent
                store::AgentContext::for_agent(agent)
            } else {
                // default: only show public entries
                store::AgentContext::public_for_agent(agent)
            }
        }
        _ => store::AgentContext::public_only(),
    }
}

/// Compute the "hidden private entries" hint for a `list`/`search` query, or
/// `None` when no hint should be shown (Issue #400).
///
/// The public-only default of `list`/`search` silently omits the caller's OWN
/// private entries (an ~85% undercount was observed), while `wake` includes
/// them. This surfaces that gap as a best-effort, stderr-only nudge — it never
/// touches stdout, `--json` output, or the exit code. This function is PURE
/// w.r.t. output: it returns the message string and lets the caller
/// (`warn_hidden_private`) do the `eprintln!`, which keeps it unit-testable.
///
/// Returns `None` (no hint) when ANY of the following hold:
///   - the search ran in `--semantic` mode (`semantic == true`): the count
///     query below matches with the BM25 `@@` text predicate, which does NOT
///     mirror vector similarity. Counting under semantic mode would produce both
///     false negatives (a semantic match with no literal term overlap goes
///     uncounted — the very #400 undercount this exists to fix) and false
///     positives (a literal `@@` match that isn't in the top-N vector results,
///     or isn't embedded at all, would promise an entry `--include-private
///     --semantic` never shows). So the hint is gated off entirely there;
///   - the context already includes private entries (`ctx.include_private`),
///     i.e. `--include-private` or `--mine` was given — those already show them;
///   - there is no calling agent (`MX_CURRENT_AGENT` unset ⇒ `agent_id` is
///     `None`), so there are no owned-private entries to hide;
///   - the count query errors — the hint is best-effort and must NEVER fail the
///     command over a diagnostic;
///   - zero owned-private entries survive the SAME in-memory filters
///     (`apply_entry_filters`, minus the display limit) the main query applies.
///
/// `query` is `Some(terms)` for `search` (matched with the same BM25 `@@`
/// full-text predicate the non-semantic `search` branch uses) and `None` for
/// `list`. `semantic` is the `search --semantic` flag (always `false` for
/// `list`); when set, the hint is suppressed per the first bullet above.
pub(crate) fn hidden_private_hint(
    db: &dyn store::KnowledgeStore,
    ctx: &store::AgentContext,
    filter: &EntryFilter,
    query: Option<&str>,
    semantic: bool,
) -> Option<String> {
    // Semantic search matches by vector similarity, but the count query below
    // uses the BM25 `@@` text predicate — the two do not agree. Rather than emit
    // a hint that could over- or under-count relative to what `--include-private
    // --semantic` would actually show, gate it off. (Counting with the semantic
    // predicate would also be heavier and top-N-dependent.)
    if semantic {
        return None;
    }

    // Trigger only in the default (public-only) view AND when there is a
    // calling agent whose private entries could exist. `--include-private` and
    // `--mine` both resolve to `include_private = true`, so they no-op here;
    // an anonymous/no-agent context has `agent_id == None` and also no-ops.
    if ctx.include_private {
        return None;
    }
    let agent = ctx.agent_id.as_deref()?;

    // Same category + resonance filter the main query builds. Category is
    // sourced from the flag (list handles categories in the handler; here we
    // fold them into the DB filter so build_category_filter matches all of
    // them at once — equivalent to the union of the per-category queries).
    let db_filter = store::KnowledgeFilter {
        min_resonance: filter.min_resonance,
        max_resonance: filter.max_resonance,
        categories: filter.category.clone(),
    };

    // Best-effort: on ANY error, stay silent. The hint must never turn a
    // successful list/search into a failure (kn-97344000: don't trade one
    // silent-data defect for a louder one).
    let candidates = db.owned_private_matching(agent, query, &db_filter).ok()?;

    // Apply the SAME in-memory filters the main query uses (tags + field
    // presence), but with the display `limit` stripped: the hint reports how
    // many owned-private matches are hidden in total, not just the first page.
    let mut filter_no_limit = filter.clone();
    filter_no_limit.limit = None;
    let count = apply_entry_filters(candidates, &filter_no_limit).len();

    if count == 0 {
        return None;
    }

    let (noun, verb) = if count == 1 {
        ("entry", "is")
    } else {
        ("entries", "are")
    };
    Some(format!(
        "note: {count} private {noun} of yours matched but {verb} hidden; \
         use --include-private to see them"
    ))
}

/// Print the [`hidden_private_hint`] to STDERR when one applies (Issue #400).
///
/// Side-effect wrapper: STDERR only, so it can never alter stdout, `--json`
/// output, or the exit code. No-ops entirely when `hidden_private_hint`
/// returns `None`.
pub(crate) fn warn_hidden_private(
    db: &dyn store::KnowledgeStore,
    ctx: &store::AgentContext,
    filter: &EntryFilter,
    query: Option<&str>,
    semantic: bool,
) {
    if let Some(msg) = hidden_private_hint(db, ctx, filter, query, semantic) {
        eprintln!("{msg}");
    }
}

/// Similarity threshold above which two entries are considered near-duplicates
/// and should NOT be anchored together. Used in both the batch `AutoAnchor`
/// handler and the per-entry `auto_anchor` helper.
pub(crate) const NEAR_DUPLICATE_CEILING: f32 = 0.95;

/// Default minimum similarity for two entries to be considered anchor-worthy.
pub(crate) const DEFAULT_ANCHOR_THRESHOLD: f32 = 0.75;

/// Over-fetch factor for `auto_anchor`'s bounded candidate query (Issue #362):
/// we fetch `MAX_ANCHORS * ANCHOR_CANDIDATE_OVERFETCH` rows by score, leaving
/// headroom for the handful of high-scoring rows that get filtered out (self,
/// near-duplicates above the ceiling, existing/removed anchors) before we run
/// out of in-band candidates.
pub(crate) const ANCHOR_CANDIDATE_OVERFETCH: usize = 5;

/// Escalation cap for `auto_anchor`'s candidate query. When the normal
/// over-fetch is *saturated* in a way that could truncate genuine in-band
/// anchors (see the saturation signal at the call site, Issue #362 / PR #366),
/// we re-query at this much larger bound so the selected anchors provably match
/// the old exhaustive full-scan behavior. A single re-query at this cap is still
/// far cheaper than the old per-write full hydrate + Rust cosine loop, and it
/// only fires in the degenerate near-duplicate-flood case.
pub(crate) const MAX_ANCHOR_CANDIDATES: usize = 500;

/// Calculate cosine similarity between two vectors
///
/// Returns a value between -1.0 and 1.0 (typically 0.0 to 1.0 for normalized embeddings)
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }

    dot_product / (magnitude_a * magnitude_b)
}

/// Auto-embed a knowledge entry using a pre-constructed provider and tokenizer.
///
/// This is the core embedding implementation. It accepts an already-constructed
/// `TractProvider` and tokenizer so a batch caller can hoist model construction
/// ONCE above a loop and amortize the ~435 MB cold-load across N entries.
///
/// For short entries (<=400 tokens): stores a single embedding on the entry.
/// For long entries (>400 tokens): splits into overlapping chunks, embeds each
/// chunk separately, stores chunks in `embedding_chunk` table, and stores a
/// mean vector on the entry for auto_anchor compatibility.
pub(crate) fn auto_embed_with(
    entry_id: &str,
    db: &dyn store::KnowledgeStore,
    provider: &crate::embeddings::TractProvider,
    chunking_tokenizer: &tokenizers::Tokenizer,
) -> Result<()> {
    use crate::chunking::{ChunkConfig, chunk_text};
    use crate::embeddings::EmbeddingProvider;

    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
        _ => store::AgentContext::public_only(),
    };

    let mut entry = match db.get(entry_id, &ctx)? {
        Some(e) => e,
        None => return Ok(()),
    };

    let embedding_text = entry.embedding_text();
    let config = ChunkConfig::default();
    // Use load_tokenizer() (no truncation) for chunking — the provider's
    // tokenizer truncates at 512 which would hide content beyond that point.
    // Chunking must see ALL tokens to split them correctly.
    let chunks = chunk_text(&embedding_text, chunking_tokenizer, &config);

    if chunks.len() == 1 {
        // Short entry: single embedding, no chunks
        let embedding = provider.embed(&chunks[0].text)?;
        entry.embedding = Some(embedding);
        entry.embedding_model = Some(provider.model_id().to_string());
        entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
        entry.chunk_count = 0;
        entry.updated_at = Some(chrono::Utc::now().to_rfc3339());
        db.upsert_knowledge(&entry)?;
        db.delete_embedding_chunks(entry_id)?; // clean up any stale chunks
    } else {
        // Long entry: chunk, embed each, store chunks + mean vector on entry
        let mut chunk_embeddings = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            chunk_embeddings.push(provider.embed(&chunk.text)?);
        }

        // Store chunks (delete-then-insert)
        db.delete_embedding_chunks(entry_id)?;
        for (chunk, embedding) in chunks.iter().zip(chunk_embeddings.iter()) {
            db.insert_embedding_chunk(
                entry_id,
                chunk.chunk_index,
                &chunk.text,
                chunk.token_offset,
                chunk.token_count,
                embedding,
                provider.model_id(),
            )?;
        }

        // Mean vector on entry (for auto_anchor compatibility)
        let dims = provider.dimensions();
        let mut mean_vec = vec![0.0f32; dims];
        for emb in &chunk_embeddings {
            for (i, v) in emb.iter().enumerate() {
                mean_vec[i] += v;
            }
        }
        let n = chunk_embeddings.len() as f32;
        for v in mean_vec.iter_mut() {
            *v /= n;
        }
        // L2 normalize
        let l2: f32 = mean_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if l2 > 0.0 {
            for v in mean_vec.iter_mut() {
                *v /= l2;
            }
        }

        entry.embedding = Some(mean_vec);
        entry.embedding_model = Some(provider.model_id().to_string());
        entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
        entry.chunk_count = chunks.len() as i32;
        entry.updated_at = Some(chrono::Utc::now().to_rfc3339());
        db.upsert_knowledge(&entry)?;
    }

    Ok(())
}

/// Auto-embed a knowledge entry after add/update.
///
/// Thin wrapper around `auto_embed_with` that constructs the `TractProvider`
/// and chunking tokenizer inline. Use this on single-entry write paths
/// (Add, Update, Edit, Append, Prepend, Restore) where paying one model
/// cold-load per call is acceptable.
///
/// For batch callers that need to amortize the ~435 MB model cold-load across
/// N entries, use `auto_embed_with` directly: construct the provider and
/// tokenizer once, then call `auto_embed_with` in the loop.
pub(crate) fn auto_embed(entry_id: &str, db: &dyn store::KnowledgeStore) -> Result<()> {
    use crate::embeddings::TractProvider;
    let provider = TractProvider::new()?;
    let chunking_tokenizer = crate::embeddings::load_tokenizer()?;
    auto_embed_with(entry_id, db, &provider, &chunking_tokenizer)
}

/// Whether the write path should run `auto_anchor` synchronously after a
/// mutation (Add/Update/Edit/Append/Prepend/Restore).
///
/// Anchoring on the write path is disabled when EITHER:
///   - the caller passed `--no-auto-anchor` (`no_auto_anchor == true`), or
///   - `MX_SKIP_WRITE_ANCHOR` is set to `1`/`true` (case-insensitive).
///
/// The env-var parsing mirrors the `MX_SKIP_SCHEMA` convention
/// (`connection.rs`) so the project keeps one rule for boolean opt-out flags.
///
/// `MX_SKIP_WRITE_ANCHOR` is a future-facing opt-out: it lets a deployment
/// defer anchoring entirely to the explicit `mx memory auto-anchor` batch
/// command (e.g. a nightly cron), which is never gated by this flag.
///
/// Skipping anchoring does NOT affect durability. By the time this gate is
/// evaluated the entry has already been `upsert_knowledge`d, read-back
/// verified (the Add path `bail!`s if the row is absent), and re-upserted by
/// `auto_embed` — so the write is provably durable before anchoring would
/// ever run. `auto_anchor` itself returns early WITHOUT any upsert whenever
/// an entry has no embedding or no in-band neighbours; its trailing upsert is
/// an anchor update, not a load-bearing commit. Hence skipping it loses
/// anchors-for-this-write, nothing else.
pub(crate) fn write_anchor_enabled(no_auto_anchor: bool) -> bool {
    let skip_via_env =
        std::env::var("MX_SKIP_WRITE_ANCHOR").is_ok_and(|v| v == "1" || v.to_lowercase() == "true");
    !no_auto_anchor && !skip_via_env
}

/// Whether the write path should run `auto_embed` synchronously after a
/// mutation (Add/Update/Edit/Append/Prepend/Restore).
///
/// Embedding on the write path is disabled when EITHER:
///   - the caller passed `--no-embed` (`no_embed == true`), or
///   - `MX_SKIP_WRITE_EMBED` is set to `1`/`true` (case-insensitive).
///
/// The env-var parsing mirrors the `MX_SKIP_SCHEMA` convention
/// (`connection.rs`) so the project keeps one rule for boolean opt-out flags.
///
/// `MX_SKIP_WRITE_EMBED` is a deployment opt-out: it lets a caller defer
/// embedding entirely to the explicit `mx memory embed --all` batch command
/// (e.g. a nightly cron), which is never gated by this flag.
///
/// Skipping embedding does NOT affect durability or keyword/tag search.
/// By the time this gate is evaluated the entry has already been
/// `upsert_knowledge`d and read-back verified, so the write is provably
/// durable. Entries written with `--no-embed` appear in keyword and tag
/// searches but are absent from `--semantic` (vector) search results until
/// `mx memory embed --all` runs.
pub(crate) fn write_embed_enabled(no_embed: bool) -> bool {
    let skip_via_env =
        std::env::var("MX_SKIP_WRITE_EMBED").is_ok_and(|v| v == "1" || v.to_lowercase() == "true");
    !no_embed && !skip_via_env
}

/// Auto-anchor a knowledge entry after add/update
///
/// This silently finds similar entries and adds anchors for a single entry.
/// Uses defaults: threshold 0.75, max 5 anchors.
pub(crate) fn auto_anchor(
    entry_id: &str,
    db: &dyn store::KnowledgeStore,
    explicitly_removed: Option<&[String]>,
) -> Result<()> {
    // Get agent context for fetching entries
    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
        _ => store::AgentContext::public_only(),
    };

    // Fetch the entry
    let entry = match db.get(entry_id, &ctx)? {
        Some(e) => e,
        None => return Ok(()), // Entry not found, skip silently
    };

    // Skip if no embedding
    if entry.embedding.is_none() {
        return Ok(());
    }

    let entry_embedding = entry.embedding.as_ref().unwrap();

    let threshold = DEFAULT_ANCHOR_THRESHOLD;
    let max_anchors = 5;

    // ---- Candidate fetch (Issue #362) -----------------------------------
    //
    // Previously this hydrated the ENTIRE graph (`db.list_all`) and ran a Rust
    // cosine loop over every embedded entry on every write — O(n) in the
    // application layer, the dominant cost of a save (~15.6s of ~16s).
    //
    // Instead we ask the DB for the top-K most similar entries by cosine score,
    // scored against this entry's own embedding (entry-level mean vector only —
    // see `semantic_search_entries_scored`, which deliberately ignores
    // chunk-level matching so anchoring keeps strict entry-level-mean semantics
    // and selects the SAME anchors the old full scan would have). The band
    // filter, privacy filter, self-exclusion and max_anchors cap below are
    // applied unchanged — only the candidate SOURCE moved from full-scan to
    // bounded DB query.
    //
    // Over-fetch factor: the band is [0.75, 0.95]. The top-K-by-score query
    // returns the highest scores first, so the slots ahead of an in-band
    // candidate can be consumed by: self (~1.0), near-duplicates (>0.95),
    // existing anchors (re-handled separately below), and explicitly-removed
    // anchors. Over-fetching 5x max_anchors (25) leaves ample headroom for that
    // handful of high-scoring rejects before we run out of in-band candidates.
    let candidate_fetch_k = max_anchors * ANCHOR_CANDIDATE_OVERFETCH;
    let mut scored_candidates =
        db.semantic_search_entries_scored(entry_embedding, &ctx, candidate_fetch_k)?;

    // ---- Saturation detection + escalation (PR #366, hardening #362) -----
    //
    // The bounded top-K fetch diverges from the old exhaustive scan in EXACTLY
    // one degenerate case: if MORE than (K - max_anchors) rows score above an
    // in-band member (e.g. >20 near-identical copies above the 0.95 ceiling),
    // a legitimate in-band anchor could rank below K and be silently dropped —
    // a behavior change vs. the old full scan.
    //
    // We detect this with an EXACT signal. We are only at risk of having
    // truncated in-band rows when BOTH hold:
    //   1. the result is K-saturated (`len == candidate_fetch_k`), i.e. the DB
    //      had at least K candidates and the query hit its bound; AND
    //   2. the lowest-scoring returned candidate still scores at/above the band
    //      floor (>= threshold) — so scores had NOT yet dropped below 0.75 when
    //      the bound cut us off, meaning additional in-band rows may exist past K.
    //
    // This is exact: if the lowest returned score is already below the floor,
    // every row beyond K scores even lower (results are score-descending) and is
    // therefore out-of-band — nothing the old scan would have kept was missed.
    // Likewise, if the result is not saturated, the DB returned every candidate
    // it had, identical to the full scan's candidate universe.
    //
    // On the saturated signal we ESCALATE: re-query at MAX_ANCHOR_CANDIDATES and
    // proceed with that fuller set. This stays entirely on the bounded DB path
    // (no per-write full hydrate) and only triggers in the degenerate flood.
    let saturated = scored_candidates.len() == candidate_fetch_k
        && scored_candidates
            .last()
            .is_some_and(|(_, score)| *score >= threshold);
    if saturated {
        scored_candidates =
            db.semantic_search_entries_scored(entry_embedding, &ctx, MAX_ANCHOR_CANDIDATES)?;
    }

    let mut similarities: Vec<(String, f32)> = Vec::new();

    for (candidate, similarity) in &scored_candidates {
        // Skip self
        if candidate.id == entry.id {
            continue;
        }

        // Existing anchors are NOT considered as new candidates here. Their
        // staleness is re-evaluated separately below (by-ID recompute) so we
        // never depend on them appearing in the bounded top-K query.
        if entry.anchors.contains(&candidate.id) {
            continue;
        }

        // Skip anchors that the user explicitly removed via --anchors replacement.
        // auto_anchor is a safety net for missed connections, not an override of
        // explicit user intent.
        //
        // Defensive: current callers (Add, Update) already strip explicitly-removed
        // anchors before reaching this loop, but future call sites might not. This
        // guard ensures auto_anchor never re-adds an anchor the user chose to remove,
        // regardless of how the caller is wired.
        if let Some(removed) = explicitly_removed
            && removed.contains(&candidate.id)
        {
            continue;
        }

        // Privacy check
        let can_anchor = if entry.visibility == "private" {
            // Private can anchor to same-owner private OR public
            candidate.visibility == "public"
                || (candidate.visibility == "private" && candidate.owner == entry.owner)
        } else {
            // Public can only anchor to public
            candidate.visibility == "public"
        };

        if !can_anchor {
            continue;
        }

        // Filter by threshold, skip near-duplicates. The score is the same cosine
        // value the old Rust loop computed; we use the DB-computed score directly.
        if *similarity >= threshold && *similarity <= NEAR_DUPLICATE_CEILING {
            similarities.push((candidate.id.clone(), *similarity));
        }
    }

    // ---- #199 stale-anchor pruning --------------------------------------
    //
    // Re-evaluate EXISTING anchors and prune any that have dropped out of the
    // band on the current embeddings. Existing anchors won't necessarily appear
    // in the bounded top-K similarity query (they may now score below K, which
    // is precisely why they're stale), so we fetch each existing anchor's
    // embedding BY ID — a bounded handful — and recompute similarity directly
    // with `cosine_similarity`, exactly as the old full-scan path did. This
    // preserves #199 behavior with no dependency on the top-K candidate set.
    let mut stale_anchors: Vec<String> = Vec::new();
    for anchor_id in &entry.anchors {
        // Skip a degenerate self-anchor: never treat the entry's own id as stale
        // here (matches the old loop, which `continue`d on self before staleness).
        if *anchor_id == entry.id {
            continue;
        }
        let anchor_entry = match db.get(anchor_id, &ctx)? {
            Some(e) => e,
            // Anchor target no longer visible/exists: leave the existing
            // behavior untouched (old code only saw it if list_all returned it;
            // if it didn't, it wasn't pruned). Don't prune on absence.
            None => continue,
        };
        let Some(anchor_embedding) = anchor_entry.embedding.as_ref() else {
            // No embedding to compare against — old loop filtered these out of
            // `candidates`, so it never marked them stale. Preserve that.
            continue;
        };
        let similarity = cosine_similarity(entry_embedding, anchor_embedding);
        if similarity < threshold || similarity > NEAR_DUPLICATE_CEILING {
            stale_anchors.push(anchor_id.clone());
        }
    }

    // No similar entries found and no stale anchors to prune
    if stale_anchors.is_empty() && similarities.is_empty() {
        return Ok(());
    }

    // Sort by similarity (descending) and take top N
    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_matches: Vec<String> = similarities
        .into_iter()
        .take(max_anchors)
        .map(|(id, _)| id)
        .collect();

    // Update the entry with new anchors, filtering out stale ones
    let mut updated_anchors: Vec<String> = entry
        .anchors
        .clone()
        .into_iter()
        .filter(|a| !stale_anchors.contains(a))
        .collect();

    if let Some(removed) = explicitly_removed {
        updated_anchors.retain(|a| !removed.contains(a));
    }

    updated_anchors.extend(top_matches);
    updated_anchors.sort();
    updated_anchors.dedup();

    // Create updated entry
    let mut updated_entry = entry.clone();
    updated_entry.anchors = updated_anchors;
    updated_entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

    // Save to database
    db.upsert_knowledge(&updated_entry)?;

    Ok(())
}

/// Open the SurrealDB graph database for the given config.
pub(crate) fn open_surreal(config: &IndexConfig, verbose: bool) -> Result<SurrealDatabase> {
    let surreal_path = config.db_path.with_extension("surreal");
    SurrealDatabase::open_with_verbose(surreal_path, verbose)
}

#[cfg(test)]
mod auto_anchor_tests {
    //! Tests for `auto_anchor` after the Issue #362 rewrite (DB-side bounded
    //! candidate fetch replacing `list_all` + full-graph Rust cosine loop).
    //!
    //! The headline guarantee these tests defend: anchoring CORRECTNESS is
    //! unchanged — the band filter, privacy filter, self-exclusion, max_anchors
    //! cap and #199 stale-anchor pruning all behave exactly as the old full
    //! scan did; only the candidate SOURCE moved to the bounded DB query.
    //!
    //! Env-var sensitive (`MX_CURRENT_AGENT` drives the agent context), so these
    //! are `#[serial]` and reset the var deterministically.

    use super::*;
    use crate::knowledge::KnowledgeEntry;
    use crate::store::{AgentContext, KnowledgeStore};
    use serial_test::serial;

    /// A unit vector whose cosine similarity with the reference query vector
    /// `unit_query()` is exactly `cos` (within f32 precision). Built as
    /// `[cos, sin, 0, 0]`, which is unit-length, so cosine == dot product == cos.
    fn unit_vec(cos: f32) -> Vec<f32> {
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        vec![cos, sin, 0.0, 0.0]
    }

    /// The reference/query direction: `[1, 0, 0, 0]`.
    fn unit_query() -> Vec<f32> {
        vec![1.0, 0.0, 0.0, 0.0]
    }

    fn entry_with_embedding(
        id: &str,
        embedding: Vec<f32>,
        visibility: &str,
        owner: Option<&str>,
        anchors: Vec<String>,
    ) -> KnowledgeEntry {
        let now = chrono::Utc::now().to_rfc3339();
        KnowledgeEntry {
            id: id.to_string(),
            category_id: "test".to_string(),
            title: format!("Entry {id}"),
            body: Some("body".to_string()),
            summary: None,
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            tags: vec![],
            created_at: Some(now.clone()),
            updated_at: Some(now.clone()),
            content_hash: Some(format!("hash-{id}")),
            source_type_id: Some("manual".to_string()),
            entry_type_id: Some("primary".to_string()),
            session_id: None,
            ephemeral: false,
            content_type_id: Some("text".to_string()),
            owner: owner.map(|o| o.to_string()),
            visibility: visibility.to_string(),
            resonance: 5,
            resonance_type: Some("ephemeral".to_string()),
            last_activated: Some(now),
            activation_count: 0,
            decay_rate: 0.0,
            anchors,
            wake_phrases: vec![],
            triggers: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: Some(embedding),
            embedding_model: Some("test-model".to_string()),
            embedded_at: Some(chrono::Utc::now().to_rfc3339()),
            chunk_count: 0,
            format: "markdown".to_string(),
            effective_resonance: None,
        }
    }

    /// Clear `MX_CURRENT_AGENT` so `auto_anchor` uses `public_only` context.
    /// SAFETY: process-wide env mutation, serialized via `#[serial]`.
    fn clear_agent_env() {
        unsafe {
            std::env::remove_var("MX_CURRENT_AGENT");
        }
    }

    /// Reference implementation of the OLD candidate selection: full scan over
    /// every embedded entry, the band + privacy + self filters, sort-by-score,
    /// take max_anchors. Returns the set of NEW anchor ids the old code would
    /// have added (NOT including stale pruning — tested separately). Used to
    /// prove the rewrite picks the same anchors.
    fn reference_old_anchors(
        target: &KnowledgeEntry,
        all: &[KnowledgeEntry],
        max_anchors: usize,
    ) -> Vec<String> {
        let threshold = DEFAULT_ANCHOR_THRESHOLD;
        let target_emb = target.embedding.as_ref().unwrap();
        let mut sims: Vec<(String, f32)> = Vec::new();
        for cand in all {
            if cand.id == target.id {
                continue;
            }
            if target.anchors.contains(&cand.id) {
                continue;
            }
            let Some(cand_emb) = cand.embedding.as_ref() else {
                continue;
            };
            let can_anchor = if target.visibility == "private" {
                cand.visibility == "public"
                    || (cand.visibility == "private" && cand.owner == target.owner)
            } else {
                cand.visibility == "public"
            };
            if !can_anchor {
                continue;
            }
            let sim = cosine_similarity(target_emb, cand_emb);
            if sim >= threshold && sim <= NEAR_DUPLICATE_CEILING {
                sims.push((cand.id.clone(), sim));
            }
        }
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut ids: Vec<String> = sims
            .into_iter()
            .take(max_anchors)
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids
    }

    fn anchors_of(db: &dyn KnowledgeStore, id: &str) -> Vec<String> {
        let ctx = AgentContext::public_only();
        let mut a = db.get(id, &ctx).unwrap().unwrap().anchors;
        a.sort();
        a
    }

    #[test]
    #[serial]
    fn auto_anchor_picks_same_anchors_as_old_full_scan() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        // Seed a small graph spanning the whole similarity range relative to the
        // target's [1,0,0,0] direction.
        let target = entry_with_embedding("kn-target", unit_query(), "public", None, vec![]);
        let graph = vec![
            target.clone(),
            entry_with_embedding("kn-a", unit_vec(0.90), "public", None, vec![]), // in band
            entry_with_embedding("kn-b", unit_vec(0.85), "public", None, vec![]), // in band
            entry_with_embedding("kn-c", unit_vec(0.80), "public", None, vec![]), // in band
            entry_with_embedding("kn-d", unit_vec(0.78), "public", None, vec![]), // in band
            entry_with_embedding("kn-e", unit_vec(0.76), "public", None, vec![]), // in band (6th -> capped out)
            entry_with_embedding("kn-dup", unit_vec(0.97), "public", None, vec![]), // > ceiling
            entry_with_embedding("kn-far", unit_vec(0.60), "public", None, vec![]), // < threshold
        ];
        for e in &graph {
            db.upsert_knowledge(e).unwrap();
        }

        auto_anchor("kn-target", &db, None).unwrap();

        let got = anchors_of(&db, "kn-target");
        let expected = reference_old_anchors(&target, &graph, 5);

        assert_eq!(
            got, expected,
            "rewrite must select the same anchors as the old full scan"
        );
        // Concretely: top 5 in-band by score, dup/far excluded, self excluded.
        assert_eq!(
            got,
            vec![
                "kn-a".to_string(),
                "kn-b".to_string(),
                "kn-c".to_string(),
                "kn-d".to_string(),
                "kn-e".to_string()
            ]
        );
    }

    #[test]
    #[serial]
    fn band_filter_excludes_near_duplicates_and_below_threshold() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        let target = entry_with_embedding("kn-t", unit_query(), "public", None, vec![]);
        db.upsert_knowledge(&target).unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-dup",
            unit_vec(0.99),
            "public",
            None,
            vec![],
        ))
        .unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-low",
            unit_vec(0.50),
            "public",
            None,
            vec![],
        ))
        .unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-mid",
            unit_vec(0.85),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        auto_anchor("kn-t", &db, None).unwrap();
        let got = anchors_of(&db, "kn-t");

        assert_eq!(
            got,
            vec!["kn-mid".to_string()],
            "only the in-band entry is anchored; near-dup (>0.95) and below-threshold (<0.75) excluded"
        );
    }

    #[test]
    #[serial]
    fn max_anchors_cap_respected() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        let target = entry_with_embedding("kn-t", unit_query(), "public", None, vec![]);
        db.upsert_knowledge(&target).unwrap();
        // Seven in-band candidates with distinct, descending scores.
        let scores = [0.94, 0.92, 0.90, 0.88, 0.86, 0.84, 0.82];
        for (i, s) in scores.iter().enumerate() {
            db.upsert_knowledge(&entry_with_embedding(
                &format!("kn-c{i}"),
                unit_vec(*s),
                "public",
                None,
                vec![],
            ))
            .unwrap();
        }

        auto_anchor("kn-t", &db, None).unwrap();
        let got = anchors_of(&db, "kn-t");

        assert_eq!(got.len(), 5, "cap at max_anchors = 5");
        // The 5 highest-scoring band members.
        assert_eq!(
            got,
            vec![
                "kn-c0".to_string(),
                "kn-c1".to_string(),
                "kn-c2".to_string(),
                "kn-c3".to_string(),
                "kn-c4".to_string()
            ]
        );
    }

    #[test]
    #[serial]
    fn stale_anchor_pruned_via_by_id_recompute() {
        // #199: an existing anchor that has drifted below threshold must be
        // pruned even though it won't show up in the top-K similarity query.
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        // Existing anchor "kn-stale" now sits FAR from the target (cos 0.40),
        // so it is below the 0.75 floor and must be pruned. Crucially we seed
        // MANY closer entries so kn-stale ranks well below any top-K cutoff —
        // proving the by-ID recompute (not the top-K query) is what prunes it.
        let mut target = entry_with_embedding("kn-t", unit_query(), "public", None, vec![]);
        target.anchors = vec!["kn-stale".to_string(), "kn-keep".to_string()];
        db.upsert_knowledge(&target).unwrap();

        // kn-keep is still in band -> must survive re-eval.
        db.upsert_knowledge(&entry_with_embedding(
            "kn-keep",
            unit_vec(0.88),
            "public",
            None,
            vec![],
        ))
        .unwrap();
        // kn-stale drifted out of band -> must be pruned.
        db.upsert_knowledge(&entry_with_embedding(
            "kn-stale",
            unit_vec(0.40),
            "public",
            None,
            vec![],
        ))
        .unwrap();
        // A pile of fresh in-band neighbors that crowd the top-K.
        for i in 0..10 {
            db.upsert_knowledge(&entry_with_embedding(
                &format!("kn-n{i}"),
                unit_vec(0.80 + i as f32 * 0.001),
                "public",
                None,
                vec![],
            ))
            .unwrap();
        }

        auto_anchor("kn-t", &db, None).unwrap();
        let got = anchors_of(&db, "kn-t");

        assert!(
            !got.contains(&"kn-stale".to_string()),
            "stale anchor below threshold must be pruned (by-ID recompute)"
        );
        assert!(
            got.contains(&"kn-keep".to_string()),
            "in-band existing anchor must be preserved"
        );
    }

    #[test]
    #[serial]
    fn near_duplicate_existing_anchor_is_pruned() {
        // #199 also prunes existing anchors that drifted ABOVE the near-dup
        // ceiling (band is closed on both ends).
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        let mut target = entry_with_embedding("kn-t", unit_query(), "public", None, vec![]);
        target.anchors = vec!["kn-toodup".to_string()];
        db.upsert_knowledge(&target).unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-toodup",
            unit_vec(0.98),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        auto_anchor("kn-t", &db, None).unwrap();
        let got = anchors_of(&db, "kn-t");
        assert!(
            !got.contains(&"kn-toodup".to_string()),
            "existing anchor above the near-duplicate ceiling must be pruned"
        );
    }

    #[test]
    #[serial]
    fn self_is_never_anchored() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();
        let target = entry_with_embedding("kn-solo", unit_query(), "public", None, vec![]);
        db.upsert_knowledge(&target).unwrap();
        // A single in-band neighbor so anchoring runs.
        db.upsert_knowledge(&entry_with_embedding(
            "kn-near",
            unit_vec(0.85),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        auto_anchor("kn-solo", &db, None).unwrap();
        let got = anchors_of(&db, "kn-solo");
        assert!(
            !got.contains(&"kn-solo".to_string()),
            "an entry must never anchor to itself (self-similarity ~1.0 excluded)"
        );
        assert_eq!(got, vec!["kn-near".to_string()]);
    }

    #[test]
    #[serial]
    fn public_entry_does_not_anchor_to_private() {
        // Privacy preserved: a public entry must never anchor to a private one.
        // Under public_only context the DB visibility filter drops the private
        // row entirely, so it's not even a candidate.
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        let target = entry_with_embedding("kn-pub", unit_query(), "public", None, vec![]);
        db.upsert_knowledge(&target).unwrap();
        // Private candidate that WOULD be in-band by score.
        db.upsert_knowledge(&entry_with_embedding(
            "kn-priv",
            unit_vec(0.90),
            "private",
            Some("agent-x"),
            vec![],
        ))
        .unwrap();
        // A public in-band candidate so the run produces something.
        db.upsert_knowledge(&entry_with_embedding(
            "kn-pub2",
            unit_vec(0.85),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        auto_anchor("kn-pub", &db, None).unwrap();
        let got = anchors_of(&db, "kn-pub");
        assert!(
            !got.contains(&"kn-priv".to_string()),
            "public entry must not anchor to a private entry"
        );
        assert_eq!(got, vec!["kn-pub2".to_string()]);
    }

    #[test]
    #[serial]
    fn explicitly_removed_anchor_not_readded() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();
        let target = entry_with_embedding("kn-t", unit_query(), "public", None, vec![]);
        db.upsert_knowledge(&target).unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-removed",
            unit_vec(0.90),
            "public",
            None,
            vec![],
        ))
        .unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-other",
            unit_vec(0.85),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        let removed = vec!["kn-removed".to_string()];
        auto_anchor("kn-t", &db, Some(&removed)).unwrap();
        let got = anchors_of(&db, "kn-t");
        assert!(
            !got.contains(&"kn-removed".to_string()),
            "auto_anchor must not re-add an explicitly removed anchor"
        );
        assert_eq!(got, vec!["kn-other".to_string()]);
    }

    #[test]
    #[serial]
    fn no_embedding_skips_anchoring() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();
        // Build a target with NO embedding (simulates the opt-out / un-embedded
        // path: auto_anchor returns early and never fetches candidates).
        let mut target = entry_with_embedding("kn-noemb", unit_query(), "public", None, vec![]);
        target.embedding = None;
        db.upsert_knowledge(&target).unwrap();
        db.upsert_knowledge(&entry_with_embedding(
            "kn-x",
            unit_vec(0.90),
            "public",
            None,
            vec![],
        ))
        .unwrap();

        auto_anchor("kn-noemb", &db, None).unwrap();
        let got = anchors_of(&db, "kn-noemb");
        assert!(got.is_empty(), "no embedding -> no anchoring");
    }

    /// PR #366 hardening: the degenerate near-duplicate-flood case. When MORE
    /// than (K - max_anchors) entries score above the band ceiling, the initial
    /// bounded top-K (= max_anchors * ANCHOR_CANDIDATE_OVERFETCH = 25) is filled
    /// almost entirely by out-of-band near-duplicates, pushing genuine in-band
    /// anchors past slot K. Without escalation those in-band anchors would be
    /// silently dropped vs. the old exhaustive scan. With saturation detection +
    /// escalation to MAX_ANCHOR_CANDIDATES, auto_anchor must still select exactly
    /// the in-band anchors the full-scan reference impl would.
    #[test]
    #[serial]
    fn escalates_when_saturated_by_near_duplicate_flood() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        // Sanity: the over-fetch K used by auto_anchor.
        let max_anchors = 5usize;
        let k = max_anchors * ANCHOR_CANDIDATE_OVERFETCH; // 25

        let target = entry_with_embedding("kn-target", unit_query(), "public", None, vec![]);
        let mut graph = vec![target.clone()];

        // Flood: K-1 (24) near-duplicates ABOVE the 0.95 ceiling. They are NOT
        // anchorable (band-excluded), but they crowd the score-descending top-K,
        // leaving only a single slot for an in-band member in the initial fetch.
        let flood = k - 1; // 24 > (K - max_anchors) = 20
        for i in 0..flood {
            // Distinct scores in (0.95, 1.0) so ordering is deterministic and
            // every one sits strictly above the ceiling.
            let cos = 0.999 - (i as f32) * 0.0005;
            graph.push(entry_with_embedding(
                &format!("kn-dup{i:02}"),
                unit_vec(cos),
                "public",
                None,
                vec![],
            ));
        }

        // Genuine in-band anchors, all scoring BELOW every duplicate (so they
        // rank past slot K and would be truncated without escalation), but well
        // inside the [0.75, 0.95] band and within MAX_ANCHOR_CANDIDATES.
        let in_band = [
            ("kn-real-a", 0.90f32),
            ("kn-real-b", 0.87),
            ("kn-real-c", 0.84),
            ("kn-real-d", 0.81),
            ("kn-real-e", 0.78),
        ];
        for (id, cos) in in_band {
            graph.push(entry_with_embedding(
                id,
                unit_vec(cos),
                "public",
                None,
                vec![],
            ));
        }

        for e in &graph {
            db.upsert_knowledge(e).unwrap();
        }

        // Confirm the precondition: the INITIAL bounded fetch is saturated AND
        // its lowest returned score is still at/above the floor — i.e. it would
        // have truncated in-band rows without escalation.
        let ctx = AgentContext::public_only();
        let initial = db
            .semantic_search_entries_scored(target.embedding.as_ref().unwrap(), &ctx, k)
            .unwrap();
        assert_eq!(initial.len(), k, "initial fetch must be K-saturated");
        assert!(
            initial.last().unwrap().1 >= DEFAULT_ANCHOR_THRESHOLD,
            "lowest returned score must still be >= floor (saturation signal fires)"
        );

        auto_anchor("kn-target", &db, None).unwrap();

        let got = anchors_of(&db, "kn-target");
        let expected = reference_old_anchors(&target, &graph, max_anchors);

        // Escalation must recover the full in-band set, matching the exhaustive
        // reference exactly.
        assert_eq!(
            got, expected,
            "escalation must select the same in-band anchors as the old full scan"
        );
        assert_eq!(
            got,
            vec![
                "kn-real-a".to_string(),
                "kn-real-b".to_string(),
                "kn-real-c".to_string(),
                "kn-real-d".to_string(),
                "kn-real-e".to_string(),
            ],
            "all five genuine in-band anchors recovered despite the near-duplicate flood"
        );
    }

    /// PR #366: the saturation signal is EXACT — escalation must NOT fire when
    /// the bounded fetch is full but its lowest returned score is already below
    /// the band floor. In that case every row beyond K is out-of-band, so the
    /// top-K already contains every anchor-worthy candidate; re-querying would be
    /// wasted work and a perf regression in a common shape (many low-similarity
    /// neighbors). We assert correctness is preserved without relying on the
    /// escalation path.
    #[test]
    #[serial]
    fn does_not_escalate_when_lowest_score_below_floor() {
        clear_agent_env();
        let db = SurrealDatabase::open_in_memory().unwrap();

        let max_anchors = 5usize;
        let k = max_anchors * ANCHOR_CANDIDATE_OVERFETCH; // 25

        let target = entry_with_embedding("kn-target", unit_query(), "public", None, vec![]);
        let mut graph = vec![target.clone()];

        // A few in-band anchors at the top...
        let in_band = [("kn-a", 0.90f32), ("kn-b", 0.85), ("kn-c", 0.80)];
        for (id, cos) in in_band {
            graph.push(entry_with_embedding(
                id,
                unit_vec(cos),
                "public",
                None,
                vec![],
            ));
        }
        // ...then MANY below-floor neighbors so the fetch is K-saturated but its
        // tail has already dropped under 0.75. (K + a margin of below-floor rows.)
        for i in 0..(k + 10) {
            let cos = 0.70 - (i as f32) * 0.001; // all strictly below 0.75 floor
            graph.push(entry_with_embedding(
                &format!("kn-lo{i:02}"),
                unit_vec(cos),
                "public",
                None,
                vec![],
            ));
        }

        for e in &graph {
            db.upsert_knowledge(e).unwrap();
        }

        // Precondition: K-saturated, but lowest returned score is BELOW the floor
        // -> the exact signal says "no truncation possible", escalation suppressed.
        let ctx = AgentContext::public_only();
        let initial = db
            .semantic_search_entries_scored(target.embedding.as_ref().unwrap(), &ctx, k)
            .unwrap();
        assert_eq!(initial.len(), k, "fetch is K-saturated");
        assert!(
            initial.last().unwrap().1 < DEFAULT_ANCHOR_THRESHOLD,
            "lowest returned score is below floor -> escalation must NOT fire"
        );

        auto_anchor("kn-target", &db, None).unwrap();

        let got = anchors_of(&db, "kn-target");
        assert_eq!(
            got,
            vec!["kn-a".to_string(), "kn-b".to_string(), "kn-c".to_string()],
            "the three in-band anchors are selected from the initial top-K (no escalation needed)"
        );
    }

    // =====================================================================
    // MX_SKIP_WRITE_ANCHOR opt-out (PR #364)
    //
    // Two things under test:
    //   1. write_anchor_enabled — the single source of truth for the gate.
    //      Tested directly (not a re-implemented copy of the condition) for
    //      every accepted value of the flag plus the --no-auto-anchor flag.
    //   2. Durability — a write whose anchoring is skipped (flag ON) still
    //      persists. Proven against a REAL file-backed store across a
    //      drop+reopen, since an in-memory store cannot demonstrate
    //      durability across a fresh connection.
    //
    // commit_entry was REMOVED in this PR: post-#362 the Add path upserts +
    // read-back-verifies + auto_embeds (which upserts again) BEFORE the
    // anchor step, so the entry is provably durable before auto_anchor would
    // run. auto_anchor also returns early WITHOUT upserting whenever an entry
    // has no embedding or no in-band neighbours, so its trailing upsert is an
    // anchor update, not a load-bearing commit. The skip path therefore needs
    // no extra upsert.
    // =====================================================================

    /// Save the current MX_SKIP_WRITE_ANCHOR value, set it (or clear it),
    /// evaluate the gate, then restore — so the env state never leaks.
    /// SAFETY: process-wide env mutation, serialized via `#[serial]`.
    fn gate_with_env(value: Option<&str>, no_auto_anchor: bool) -> bool {
        let prev = std::env::var("MX_SKIP_WRITE_ANCHOR").ok();
        unsafe {
            match value {
                Some(v) => std::env::set_var("MX_SKIP_WRITE_ANCHOR", v),
                None => std::env::remove_var("MX_SKIP_WRITE_ANCHOR"),
            }
        }
        let enabled = write_anchor_enabled(no_auto_anchor);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_SKIP_WRITE_ANCHOR", v),
                None => std::env::remove_var("MX_SKIP_WRITE_ANCHOR"),
            }
        }
        enabled
    }

    #[test]
    #[serial]
    fn write_anchor_enabled_unset_flag_runs_anchoring() {
        assert!(
            gate_with_env(None, false),
            "unset MX_SKIP_WRITE_ANCHOR must leave anchoring ON (default behavior preserved)"
        );
    }

    #[test]
    #[serial]
    fn write_anchor_enabled_flag_1_skips_anchoring() {
        assert!(
            !gate_with_env(Some("1"), false),
            "MX_SKIP_WRITE_ANCHOR=1 must turn write-path anchoring OFF"
        );
    }

    #[test]
    #[serial]
    fn write_anchor_enabled_flag_true_skips_anchoring() {
        assert!(
            !gate_with_env(Some("true"), false),
            "MX_SKIP_WRITE_ANCHOR=true must turn write-path anchoring OFF"
        );
        assert!(
            !gate_with_env(Some("TRUE"), false),
            "MX_SKIP_WRITE_ANCHOR is case-insensitive for 'true'"
        );
    }

    #[test]
    #[serial]
    fn write_anchor_enabled_other_values_run_anchoring() {
        // Only "1"/"true" opt out; anything else (incl. "0", "false", "yes")
        // leaves anchoring on, matching the MX_SKIP_SCHEMA convention.
        assert!(gate_with_env(Some("0"), false), "'0' must not opt out");
        assert!(
            gate_with_env(Some("false"), false),
            "'false' must not opt out"
        );
        assert!(gate_with_env(Some(""), false), "empty must not opt out");
    }

    #[test]
    #[serial]
    fn write_anchor_enabled_cli_flag_always_skips() {
        // --no-auto-anchor closes the gate regardless of the env var.
        assert!(
            !gate_with_env(None, true),
            "--no-auto-anchor must skip anchoring even with the env flag unset"
        );
        assert!(
            !gate_with_env(Some("0"), true),
            "--no-auto-anchor must skip anchoring even when env flag would allow it"
        );
    }

    #[test]
    #[serial]
    fn skipped_anchor_write_persists_across_reopen() {
        // The honest durability test: with anchoring skipped (flag ON), a
        // write must survive being dropped and re-opened from a REAL
        // file-backed store. This is what the write path actually does —
        // upsert_knowledge — minus the auto_anchor step the flag removes.
        //
        // An in-memory store cannot prove this (it dies with the handle), so
        // we use a file-backed store against a tempdir path and reopen it.
        //
        // CRITICAL: we go through `open_file_backed_for_test`, NOT the plain
        // `SurrealDatabase::open`. `open` reads `MX_SURREAL_*` env, and if the
        // ambient shell sets `MX_SURREAL_MODE=network` the explicit tempdir
        // path is ignored and the write lands on the LIVE database — which is
        // exactly how a dim-4 fixture once poisoned every production cosine
        // scan. `open_file_backed_for_test` forces an embedded store at the
        // tempdir and asserts the endpoint is local before any write.
        clear_agent_env();
        let prev = std::env::var("MX_SKIP_WRITE_ANCHOR").ok();
        unsafe { std::env::set_var("MX_SKIP_WRITE_ANCHOR", "1") };

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("durability.surreal");

        // Precondition: the gate is closed, i.e. the handler would NOT call
        // auto_anchor — only the plain write happens.
        assert!(
            !write_anchor_enabled(false),
            "precondition: flag=1 must skip anchoring"
        );

        // Write phase: open, upsert, drop the handle (simulating process exit).
        {
            let db = SurrealDatabase::open_file_backed_for_test(&db_path).unwrap();
            let entry =
                entry_with_embedding("kn-skip-durable", unit_query(), "public", None, vec![]);
            db.upsert_knowledge(&entry).unwrap();
        }

        // Reopen phase: a brand-new connection to the same on-disk store.
        let reopened = SurrealDatabase::open_file_backed_for_test(&db_path).unwrap();
        let ctx = AgentContext::public_only();
        let got = reopened.get("kn-skip-durable", &ctx).unwrap();

        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_SKIP_WRITE_ANCHOR", v),
                None => std::env::remove_var("MX_SKIP_WRITE_ANCHOR"),
            }
        }

        assert!(
            got.is_some(),
            "a write with anchoring skipped must persist across a drop+reopen (no commit_entry needed)"
        );
    }

    // =====================================================================
    // MX_SKIP_WRITE_EMBED opt-out
    //
    // Mirrors the MX_SKIP_WRITE_ANCHOR tests above. `write_embed_enabled` is
    // the single source of truth for the embed gate; these tests cover every
    // accepted value plus the `--no-embed` CLI flag.
    // =====================================================================

    /// Save the current MX_SKIP_WRITE_EMBED value, set it (or clear it),
    /// evaluate the gate, then restore — so the env state never leaks.
    /// SAFETY: process-wide env mutation, serialized via `#[serial]`.
    fn embed_gate_with_env(value: Option<&str>, no_embed: bool) -> bool {
        let prev = std::env::var("MX_SKIP_WRITE_EMBED").ok();
        unsafe {
            match value {
                Some(v) => std::env::set_var("MX_SKIP_WRITE_EMBED", v),
                None => std::env::remove_var("MX_SKIP_WRITE_EMBED"),
            }
        }
        let enabled = write_embed_enabled(no_embed);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_SKIP_WRITE_EMBED", v),
                None => std::env::remove_var("MX_SKIP_WRITE_EMBED"),
            }
        }
        enabled
    }

    #[test]
    #[serial]
    fn write_embed_enabled_unset_flag_runs_embedding() {
        assert!(
            embed_gate_with_env(None, false),
            "unset MX_SKIP_WRITE_EMBED must leave embedding ON (default behavior preserved)"
        );
    }

    #[test]
    #[serial]
    fn write_embed_enabled_flag_1_skips_embedding() {
        assert!(
            !embed_gate_with_env(Some("1"), false),
            "MX_SKIP_WRITE_EMBED=1 must turn write-path embedding OFF"
        );
    }

    #[test]
    #[serial]
    fn write_embed_enabled_flag_true_skips_embedding() {
        assert!(
            !embed_gate_with_env(Some("true"), false),
            "MX_SKIP_WRITE_EMBED=true must turn write-path embedding OFF"
        );
        assert!(
            !embed_gate_with_env(Some("TRUE"), false),
            "MX_SKIP_WRITE_EMBED is case-insensitive for 'true'"
        );
    }

    #[test]
    #[serial]
    fn write_embed_enabled_other_values_run_embedding() {
        // Only "1"/"true" opt out; anything else (incl. "0", "false", "yes")
        // leaves embedding on, matching the MX_SKIP_SCHEMA convention.
        assert!(
            embed_gate_with_env(Some("0"), false),
            "'0' must not opt out"
        );
        assert!(
            embed_gate_with_env(Some("false"), false),
            "'false' must not opt out"
        );
        assert!(
            embed_gate_with_env(Some(""), false),
            "empty must not opt out"
        );
    }

    #[test]
    #[serial]
    fn write_embed_enabled_cli_flag_always_skips() {
        // --no-embed closes the gate regardless of the env var.
        assert!(
            !embed_gate_with_env(None, true),
            "--no-embed must skip embedding even with the env flag unset"
        );
        assert!(
            !embed_gate_with_env(Some("0"), true),
            "--no-embed must skip embedding even when env flag would allow it"
        );
    }
}

#[cfg(test)]
mod hidden_private_hint_tests {
    //! Issue #400: the stderr hint that surfaces the caller's OWN private
    //! entries when list/search hide them behind the public-only default.
    //!
    //! `hidden_private_hint` is pure w.r.t. output (returns the message or
    //! `None`), so these tests assert the trigger matrix and message text
    //! directly. STDOUT/JSON invariance is structural: the hint value only ever
    //! reaches the terminal via `warn_hidden_private`'s `eprintln!` (STDERR),
    //! and the handlers call it AFTER all stdout/JSON printing — no code path
    //! lets it touch stdout.

    use super::*;
    use crate::cli::EntryFilter;
    use crate::knowledge::KnowledgeEntry;
    use crate::store::{AgentContext, KnowledgeStore};
    use crate::surreal_db::SurrealDatabase;
    use serial_test::serial;

    /// A baseline `EntryFilter` with every flag off / unset. Tests tweak the
    /// one field under test.
    fn base_filter() -> EntryFilter {
        EntryFilter {
            category: None,
            json: false,
            mine: false,
            include_private: false,
            min_resonance: None,
            max_resonance: None,
            has_wake_phrase: false,
            missing_wake_phrase: false,
            has_anchors: false,
            missing_anchors: false,
            has_resonance_type: false,
            missing_resonance_type: false,
            limit: None,
            tags: None,
        }
    }

    /// An owned-private entry for `owner`, with the given id/title/body/tags.
    fn priv_entry(
        id: &str,
        owner: &str,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> KnowledgeEntry {
        let now = chrono::Utc::now().to_rfc3339();
        KnowledgeEntry {
            id: id.to_string(),
            category_id: "test".to_string(),
            title: title.to_string(),
            body: Some(body.to_string()),
            summary: None,
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            tags,
            created_at: Some(now.clone()),
            updated_at: Some(now.clone()),
            content_hash: Some(format!("hash-{id}")),
            source_type_id: Some("manual".to_string()),
            entry_type_id: Some("primary".to_string()),
            session_id: None,
            ephemeral: false,
            content_type_id: Some("text".to_string()),
            owner: Some(owner.to_string()),
            visibility: "private".to_string(),
            resonance: 5,
            resonance_type: Some("ephemeral".to_string()),
            last_activated: Some(now),
            activation_count: 0,
            decay_rate: 0.0,
            anchors: vec![],
            wake_phrases: vec![],
            triggers: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            chunk_count: 0,
            format: "markdown".to_string(),
            effective_resonance: None,
        }
    }

    #[test]
    fn hint_appears_for_list_when_owned_private_hidden() {
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry("kn-a", "agent-a", "my note", "body", vec![]))
            .unwrap();

        // Default view for a known agent: public-only ctx, agent_id set.
        let ctx = AgentContext::public_for_agent("agent-a");
        let msg = hidden_private_hint(&db, &ctx, &base_filter(), None, false)
            .expect("hint should fire for a hidden owned-private match");

        assert!(msg.contains("1 private entry of yours"), "msg: {msg}");
        assert!(msg.contains("is hidden"), "msg: {msg}");
        assert!(msg.contains("--include-private"), "msg: {msg}");
    }

    #[test]
    fn hint_pluralizes_for_multiple_matches() {
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry("kn-a", "agent-a", "one", "b", vec![]))
            .unwrap();
        db.upsert_knowledge(&priv_entry("kn-b", "agent-a", "two", "b", vec![]))
            .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");
        let msg = hidden_private_hint(&db, &ctx, &base_filter(), None, false).unwrap();

        assert!(msg.contains("2 private entries of yours"), "msg: {msg}");
        assert!(msg.contains("are hidden"), "msg: {msg}");
    }

    #[test]
    fn hint_appears_for_search_matching_query() {
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry(
            "kn-a",
            "agent-a",
            "unique searchable widget",
            "unique searchable widget content",
            vec![],
        ))
        .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");
        let msg = hidden_private_hint(&db, &ctx, &base_filter(), Some("widget"), false)
            .expect("search hint should fire when an owned-private entry matches the query");
        assert!(msg.contains("1 private entry of yours"), "msg: {msg}");
    }

    #[test]
    fn hint_absent_under_semantic_mode() {
        // W1: `search --semantic` matches by vector similarity, but the hint's
        // count query uses the BM25 `@@` predicate. The two do not agree, so the
        // hint must be gated OFF under semantic mode — even when a literal `@@`
        // match exists (as it does here: same fixture as the firing text-search
        // case above), the semantic flag suppresses the hint entirely.
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry(
            "kn-a",
            "agent-a",
            "unique searchable widget",
            "unique searchable widget content",
            vec![],
        ))
        .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");
        // Same query that fires the hint in non-semantic mode; only `semantic`
        // differs. Proves the suppression is driven by the flag, not the data.
        assert!(
            hidden_private_hint(&db, &ctx, &base_filter(), Some("widget"), true).is_none(),
            "the hint must not fire under --semantic: BM25 count != vector match"
        );
    }

    #[test]
    fn hint_absent_for_search_when_query_does_not_match() {
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry(
            "kn-a",
            "agent-a",
            "unrelated sprocket",
            "unrelated sprocket content",
            vec![],
        ))
        .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");
        assert!(
            hidden_private_hint(&db, &ctx, &base_filter(), Some("widget"), false).is_none(),
            "no hint when the owned-private entry doesn't match the search terms"
        );
    }

    #[test]
    fn hint_absent_with_include_private_context() {
        // `--include-private` resolves to a for_agent ctx (include_private=true).
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry("kn-a", "agent-a", "note", "b", vec![]))
            .unwrap();

        let ctx = AgentContext::for_agent("agent-a");
        assert!(
            hidden_private_hint(&db, &ctx, &base_filter(), None, false).is_none(),
            "--include-private already shows private entries -> no hint"
        );
    }

    #[test]
    fn hint_absent_with_no_calling_agent() {
        // No MX_CURRENT_AGENT -> public_only ctx (agent_id = None).
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry("kn-a", "agent-a", "note", "b", vec![]))
            .unwrap();

        let ctx = AgentContext::public_only();
        assert!(
            hidden_private_hint(&db, &ctx, &base_filter(), None, false).is_none(),
            "no calling agent -> nothing owned to hide -> no hint"
        );
    }

    #[test]
    fn hint_absent_when_no_owned_private_matches() {
        // Only ANOTHER agent's private entry exists; caller has none.
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry("kn-b", "agent-b", "theirs", "b", vec![]))
            .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");
        assert!(
            hidden_private_hint(&db, &ctx, &base_filter(), None, false).is_none(),
            "caller has no owned-private matches (and must never count agent-b's) -> no hint"
        );
    }

    #[test]
    fn hint_respects_tag_filter() {
        let db = SurrealDatabase::open_in_memory().unwrap();
        db.upsert_knowledge(&priv_entry(
            "kn-a",
            "agent-a",
            "note",
            "b",
            vec!["focus".to_string()],
        ))
        .unwrap();

        let ctx = AgentContext::public_for_agent("agent-a");

        // Tag that the entry does NOT have -> filtered out in-memory -> no hint.
        let mut f = base_filter();
        f.tags = Some(vec!["nonmatch".to_string()]);
        assert!(
            hidden_private_hint(&db, &ctx, &f, None, false).is_none(),
            "tag filter must apply to the hint count exactly as to the main query"
        );

        // Matching tag -> hint fires.
        let mut f2 = base_filter();
        f2.tags = Some(vec!["focus".to_string()]);
        assert!(
            hidden_private_hint(&db, &ctx, &f2, None, false).is_some(),
            "a matching tag filter must still surface the hidden owned-private entry"
        );
    }

    #[test]
    fn hint_count_ignores_display_limit() {
        // The display `--limit` truncates the visible list but must NOT cap the
        // hint count: the hint reports the TOTAL owned-private matches hidden.
        let db = SurrealDatabase::open_in_memory().unwrap();
        for i in 0..3 {
            db.upsert_knowledge(&priv_entry(
                &format!("kn-{i}"),
                "agent-a",
                "note",
                "b",
                vec![],
            ))
            .unwrap();
        }

        let ctx = AgentContext::public_for_agent("agent-a");
        let mut f = base_filter();
        f.limit = Some(1);
        let msg = hidden_private_hint(&db, &ctx, &f, None, false).unwrap();
        assert!(
            msg.contains("3 private entries of yours"),
            "hint counts all hidden matches regardless of --limit; msg: {msg}"
        );
    }

    // --- resolve_agent_context flag -> ctx mapping (the hint's trigger seam) ---
    // These pin the mapping the hint relies on: only the DEFAULT branch (no
    // --mine, no --include-private) yields include_private=false, which is the
    // sole case that can fire the hint. --mine and --include-private both flip
    // include_private=true and thus suppress it.

    fn with_agent_env<T>(agent: Option<&str>, f: impl FnOnce() -> T) -> T {
        let prev = std::env::var("MX_CURRENT_AGENT").ok();
        unsafe {
            match agent {
                Some(a) => std::env::set_var("MX_CURRENT_AGENT", a),
                None => std::env::remove_var("MX_CURRENT_AGENT"),
            }
        }
        let out = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("MX_CURRENT_AGENT", v),
                None => std::env::remove_var("MX_CURRENT_AGENT"),
            }
        }
        out
    }

    #[test]
    #[serial]
    fn resolve_context_default_branch_is_the_only_hint_trigger() {
        with_agent_env(Some("agent-a"), || {
            let default_ctx = resolve_agent_context(false, false);
            assert!(
                !default_ctx.include_private && default_ctx.agent_id.is_some(),
                "default branch: public-only view with a known agent -> CAN trigger hint"
            );

            let mine_ctx = resolve_agent_context(true, false);
            assert!(
                mine_ctx.include_private,
                "--mine flips include_private=true -> hint suppressed"
            );

            let incl_ctx = resolve_agent_context(false, true);
            assert!(
                incl_ctx.include_private,
                "--include-private flips include_private=true -> hint suppressed"
            );
        });
    }

    #[test]
    #[serial]
    fn resolve_context_no_agent_never_triggers_hint() {
        with_agent_env(None, || {
            let ctx = resolve_agent_context(false, false);
            assert!(
                ctx.agent_id.is_none(),
                "no MX_CURRENT_AGENT -> agent_id None -> hint suppressed"
            );
        });
    }
}
