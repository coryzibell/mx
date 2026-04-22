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

/// Auto-embed a knowledge entry after add/update
///
/// This silently generates and updates the embedding for a single entry.
pub(crate) fn auto_embed(entry_id: &str, db: &dyn store::KnowledgeStore) -> Result<()> {
    use crate::embeddings::{EmbeddingProvider, FastEmbedProvider};

    // Get agent context for fetching the entry
    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
        _ => store::AgentContext::public_only(),
    };

    // Fetch the entry
    let mut entry = match db.get(entry_id, &ctx)? {
        Some(e) => e,
        None => return Ok(()), // Entry not found, skip silently
    };

    // Initialize embedding provider
    let mut provider = FastEmbedProvider::new()?;

    // Use the entry's embedding_text method (DRY - shared with other embedding paths)
    let embedding_text = entry.embedding_text();

    // Generate embedding
    let embedding = provider.embed(&embedding_text)?;

    // Update entry with embedding
    entry.embedding = Some(embedding);
    entry.embedding_model = Some(provider.model_id().to_string());
    entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
    entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

    // Save to database
    db.upsert_knowledge(&entry)?;

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

    // Get all entries with embeddings for similarity comparison
    let all_candidates = db.list_all(&ctx)?;
    let candidates: Vec<_> = all_candidates
        .into_iter()
        .filter(|e| e.embedding.is_some())
        .collect();

    // Calculate similarities
    let threshold = 0.75;
    let max_anchors = 5;
    let mut similarities: Vec<(String, f32)> = Vec::new();

    for candidate in &candidates {
        // Skip self
        if candidate.id == entry.id {
            continue;
        }

        // Skip if already an anchor
        if entry.anchors.contains(&candidate.id) {
            continue;
        }

        // Skip anchors that the user explicitly removed via --anchors replacement.
        // auto_anchor is a safety net for missed connections, not an override of
        // explicit user intent.
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

        // Calculate cosine similarity
        let candidate_embedding = candidate.embedding.as_ref().unwrap();
        let similarity = cosine_similarity(entry_embedding, candidate_embedding);

        // Filter by threshold, skip near-duplicates
        if similarity >= threshold && similarity <= 0.95 {
            similarities.push((candidate.id.clone(), similarity));
        }
    }

    // No similar entries found
    if similarities.is_empty() {
        return Ok(());
    }

    // Sort by similarity (descending) and take top N
    similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_matches: Vec<String> = similarities
        .into_iter()
        .take(max_anchors)
        .map(|(id, _)| id)
        .collect();

    // Update the entry with new anchors
    let mut updated_anchors = entry.anchors.clone();
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
