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

/// Similarity threshold above which two entries are considered near-duplicates
/// and should NOT be anchored together. Used in both the batch `AutoAnchor`
/// handler and the per-entry `auto_anchor` helper.
pub(crate) const NEAR_DUPLICATE_CEILING: f32 = 0.95;

/// Default minimum similarity for two entries to be considered anchor-worthy.
pub(crate) const DEFAULT_ANCHOR_THRESHOLD: f32 = 0.75;

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

/// Auto-embed a knowledge entry after add/update.
///
/// For short entries (<=400 tokens): stores a single embedding on the entry.
/// For long entries (>400 tokens): splits into overlapping chunks, embeds each
/// chunk separately, stores chunks in `embedding_chunk` table, and stores a
/// mean vector on the entry for auto_anchor compatibility.
pub(crate) fn auto_embed(entry_id: &str, db: &dyn store::KnowledgeStore) -> Result<()> {
    use crate::chunking::{ChunkConfig, chunk_text};
    use crate::embeddings::{EmbeddingProvider, TractProvider};

    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
        _ => store::AgentContext::public_only(),
    };

    let mut entry = match db.get(entry_id, &ctx)? {
        Some(e) => e,
        None => return Ok(()),
    };

    let provider = TractProvider::new()?;
    let embedding_text = entry.embedding_text();
    let config = ChunkConfig::default();
    // Use load_tokenizer() (no truncation) for chunking — the provider's
    // tokenizer truncates at 512 which would hide content beyond that point.
    // Chunking must see ALL tokens to split them correctly.
    let chunking_tokenizer = crate::embeddings::load_tokenizer()?;
    let chunks = chunk_text(&embedding_text, &chunking_tokenizer, &config);

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
    // Tradeoff: if a graph had MORE than (K - max_anchors) entries scoring
    // above an in-band member (e.g. >20 near-duplicates of this entry), a band
    // member ranked below K could be missed. At anchor scale (max 5) this is
    // not a realistic configuration; the cap means we only ever keep the top
    // max_anchors in-band anyway, so the highest-scoring band members — the
    // ones we'd keep — are exactly the ones K surfaces first.
    let candidate_fetch_k = max_anchors * 5;
    let scored_candidates =
        db.semantic_search_entries_scored(entry_embedding, &ctx, candidate_fetch_k)?;

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
}
