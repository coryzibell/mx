use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use surrealdb::sql::Thing;

use crate::knowledge::KnowledgeEntry;
use crate::store::WAKE_EXCLUDED_TAGS;

use super::connection::normalize_datetime;
use super::knowledge::Projection;
use super::{SurrealConnection, SurrealDatabase};

/// The first excluded tag this entry carries, by EXACT match. Prefix matching
/// would catch any future tag that merely starts with `archive`.
fn excluded_tag_of(entry: &KnowledgeEntry) -> Option<&'static str> {
    WAKE_EXCLUDED_TAGS
        .into_iter()
        .find(|excluded| entry.tags.iter().any(|tag| tag == excluded))
}

/// Drop entries carrying an excluded tag, recording each dropped id once.
/// With `include_excluded` the entries pass through and nothing is recorded.
///
/// For a layer with no quota — the `--min-resonance` path, where the whole
/// result set IS the wake set — every excluded entry genuinely cost a place.
/// Layers with a quota use `take_included` instead.
fn keep_included(
    entries: Vec<KnowledgeEntry>,
    include_excluded: bool,
    dropped: &mut HashMap<String, &'static str>,
) -> Vec<KnowledgeEntry> {
    if include_excluded {
        return entries;
    }
    entries
        .into_iter()
        .filter(|entry| match excluded_tag_of(entry) {
            Some(tag) => {
                dropped.insert(entry.id.clone(), tag);
                false
            }
            None => true,
        })
        .collect()
}

/// Walk an ordered layer, taking up to `limit` entries that carry no excluded
/// tag, and record the excluded ones passed on the way.
///
/// The tally stops with the take, which is what keeps the reported count
/// meaningful. A layer is usually fetched wider than its quota — the core
/// query widens its window to make room for exclusions, and the recent and
/// bridge queries over-fetch by 2x — so counting every excluded entry in what
/// came back would report entries ranked below the wake set, which would have
/// been left out whether or not they were tagged.
fn take_included(
    entries: impl IntoIterator<Item = KnowledgeEntry>,
    limit: usize,
    include_excluded: bool,
    dropped: &mut HashMap<String, &'static str>,
) -> Vec<KnowledgeEntry> {
    let mut kept = Vec::with_capacity(limit);
    for entry in entries {
        if kept.len() >= limit {
            break;
        }
        match excluded_tag_of(&entry) {
            Some(tag) if !include_excluded => {
                dropped.insert(entry.id.clone(), tag);
            }
            _ => kept.push(entry),
        }
    }
    kept
}

/// Per-tag counts of the distinct entries that were dropped.
fn tally_excluded(dropped: &HashMap<String, &'static str>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for tag in dropped.values() {
        *counts.entry((*tag).to_string()).or_insert(0) += 1;
    }
    counts
}

// =========================================================================
// BACKUP OPERATIONS (Issue #206)
// =========================================================================

impl SurrealDatabase {
    /// Create a pre-mutation backup of entry content
    pub fn backup_content_internal(
        &self,
        entry: &KnowledgeEntry,
        operation: &str,
        agent: Option<&str>,
    ) -> Result<String> {
        Self::runtime().block_on(self.backup_content_async(entry, operation, agent))
    }

    async fn backup_content_async(
        &self,
        entry: &KnowledgeEntry,
        operation: &str,
        agent: Option<&str>,
    ) -> Result<String> {
        let entry_id = entry.id.clone();
        let content_hash = entry.content_hash.clone().unwrap_or_default();
        let backup_id = format!(
            "{}_{}",
            entry_id.replace("kn-", ""),
            Utc::now().format("%Y%m%dT%H%M%S%.3f")
        );

        let _response = with_db!(self, db, {
            db.query(
                "CREATE type::thing('memory_backup', $backup_id) SET
                    entry_id = $entry_id,
                    title = $title,
                    body = $body,
                    content_hash = $content_hash,
                    operation = $operation,
                    source_agent = $source_agent,
                    created_at = time::now()
                ",
            )
            .bind(("backup_id", backup_id.clone()))
            .bind(("entry_id", entry_id.clone()))
            .bind(("title", entry.title.clone()))
            .bind(("body", entry.body.clone()))
            .bind(("content_hash", content_hash))
            .bind(("operation", operation.to_string()))
            .bind(("source_agent", agent.map(|s| s.to_string())))
            .await
            .context("Failed to create memory backup")
        })?;

        // Purge old backups (keep 10 per entry) — non-fatal
        let _ = self.purge_backups_async(&entry_id, 10).await;

        Ok(backup_id)
    }

    /// List backups for an entry, newest first
    pub fn list_backups_internal(&self, entry_id: &str) -> Result<Vec<crate::types::MemoryBackup>> {
        Self::runtime().block_on(self.list_backups_async(entry_id))
    }

    async fn list_backups_async(&self, entry_id: &str) -> Result<Vec<crate::types::MemoryBackup>> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT meta::id(id) AS id, entry_id, title, body, content_hash,
                        operation, source_agent, created_at
                 FROM memory_backup
                 WHERE entry_id = $entry_id
                 ORDER BY created_at DESC",
            )
            .bind(("entry_id", entry_id.to_string()))
            .await
            .context("Failed to list memory backups")
        })?;

        let backups: Vec<crate::types::MemoryBackup> = response.take(0)?;
        Ok(backups)
    }

    /// Get the most recent backup for an entry
    pub fn latest_backup_internal(
        &self,
        entry_id: &str,
    ) -> Result<Option<crate::types::MemoryBackup>> {
        Self::runtime().block_on(self.latest_backup_async(entry_id))
    }

    async fn latest_backup_async(
        &self,
        entry_id: &str,
    ) -> Result<Option<crate::types::MemoryBackup>> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT meta::id(id) AS id, entry_id, title, body, content_hash,
                        operation, source_agent, created_at
                 FROM memory_backup
                 WHERE entry_id = $entry_id
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .bind(("entry_id", entry_id.to_string()))
            .await
            .context("Failed to get latest backup")
        })?;

        let backups: Vec<crate::types::MemoryBackup> = response.take(0)?;
        Ok(backups.into_iter().next())
    }

    /// Purge old backups, keeping the most recent `keep` per entry
    pub fn purge_backups_internal(&self, entry_id: &str, keep: usize) -> Result<()> {
        Self::runtime().block_on(self.purge_backups_async(entry_id, keep))
    }

    async fn purge_backups_async(&self, entry_id: &str, keep: usize) -> Result<()> {
        // Delete backups older than the Nth newest
        let _response = with_db!(self, db, {
            db.query(
                "DELETE FROM memory_backup
                    WHERE entry_id = $entry_id
                    AND id NOT IN (
                        SELECT VALUE id FROM memory_backup
                        WHERE entry_id = $entry_id
                        ORDER BY created_at DESC
                        LIMIT $keep
                    )",
            )
            .bind(("entry_id", entry_id.to_string()))
            .bind(("keep", keep as i64))
            .await
            .context("Failed to purge old backups")
        })?;

        Ok(())
    }

    // =========================================================================
    // WAKE CASCADE - Three-layer resonance query for identity loading
    // =========================================================================

    /// Wake-up cascade: load the calling agent's identity through three layers
    /// of resonance
    pub fn wake_cascade(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
        include_excluded: bool,
    ) -> Result<crate::store::WakeCascade> {
        Self::runtime().block_on(self.wake_cascade_async(
            ctx,
            limit,
            min_resonance,
            days,
            include_excluded,
        ))
    }

    async fn wake_cascade_async(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
        include_excluded: bool,
    ) -> Result<crate::store::WakeCascade> {
        // Tag exclusion runs in Rust, on the `tags` field every cascade query
        // populates via `value_to_knowledge_entry`, and is applied as each
        // layer is taken, so an excluded entry never occupies a slot in the
        // result. Whether the layer can then FILL its quota is a separate
        // question: the core layer widens until it can, but recent and bridges
        // only over-fetch 2x, so enough exclusions in one of them leaves the
        // wake set short. `dropped` tracks distinct ids so an entry that both
        // the core and recent queries return is counted once.
        let mut dropped: HashMap<String, &'static str> = HashMap::new();

        // If min_resonance is set, use simple query for all blooms >= threshold
        if let Some(threshold) = min_resonance {
            let blooms = self.query_blooms_by_resonance(ctx, threshold).await?;
            let core = keep_included(blooms, include_excluded, &mut dropped);
            return Ok(crate::store::WakeCascade {
                core,
                recent: Vec::new(),
                bridges: Vec::new(),
                excluded: tally_excluded(&dropped),
            });
        }

        // Sequential filling: core first, then recent, then bridges
        // This ensures we get the most important blooms first

        // Layer 1: Core foundational/transformative blooms (resonance 8+).
        //
        // Widen the SQL window until it holds `limit` entries that survive the
        // exclusion, so an excluded entry never costs a kept one its slot.
        //
        // Bounded rather than fetching the whole ordered set and truncating in
        // Rust. This layer projects `Projection::Lean` (Issue #438) so it no
        // longer carries the 768-float `embedding` column, but each row still
        // costs a body plus per-row tag/applicability follow-ups
        // (`value_to_knowledge_entry`), which still grows with the graph on
        // an unbounded core query.
        //
        // Growth is geometric. Widening by exactly the number of exclusions
        // seen terminates, but a long run of excluded entries sorting above
        // everything kept costs O(E/limit) round trips, each re-fetching what
        // the last one already did, each carrying those vectors — the cost the
        // bound exists to avoid. Doubling converges in O(log E); a realistic
        // graph finishes in one or two passes either way.
        //
        // Termination: on a pass that does not break, `kept < limit` and
        // `fetched.len() == window`, so the next window is at least
        // `window * 2 >= window + 1` — strictly increasing while `window >= 1`.
        // The table is finite, so some pass returns fewer rows than it asked
        // for, sets `exhausted`, and breaks.
        //
        // `limit == 0` is the one case that does not grow: the window stays 0
        // and `LIMIT 0` returns nothing, so `exhausted` is false and the loop
        // breaks solely because the test is `kept >= limit` and `0 >= 0`.
        // Tightening that to `>` is an infinite loop on `--limit 0`.
        let mut window = limit;
        let fetched = loop {
            let fetched = self.query_core_blooms(ctx, window).await?;
            let exhausted = fetched.len() < window;
            let kept = fetched
                .iter()
                .filter(|entry| include_excluded || excluded_tag_of(entry).is_none())
                .count();
            if kept >= limit || exhausted {
                break fetched;
            }
            let excluded_here = fetched.len() - kept;
            window = (limit + excluded_here).max(window.saturating_mul(2));
        };

        // Tally only as far as the quota is filled: entries after the last one
        // taken were scanned by a widened window, not passed over for a slot.
        let core = take_included(fetched, limit, include_excluded, &mut dropped);
        let remaining = limit.saturating_sub(core.len());

        // Layer 2: Recent blooms (last N days)
        // Exclude IDs already in core, use remaining quota
        let core_ids: HashSet<String> = core.iter().map(|e| e.id.clone()).collect();

        let all_recent = self.query_recent_blooms(ctx, remaining * 2, days).await?;
        let recent = take_included(
            all_recent.into_iter().filter(|e| !core_ids.contains(&e.id)),
            remaining,
            include_excluded,
            &mut dropped,
        );
        let remaining = remaining.saturating_sub(recent.len());

        // Layer 3: Bridge blooms (anchored to core/recent, resonance 5+).
        // An excluded entry anchored to a core entry would otherwise come back
        // here after being dropped from its own layer.
        let mut anchor_ids: Vec<String> = core
            .iter()
            .chain(recent.iter())
            .map(|e| e.id.clone())
            .collect();

        // Deduplicate anchor IDs
        anchor_ids.sort();
        anchor_ids.dedup();

        let bridges = if anchor_ids.is_empty() || remaining == 0 {
            Vec::new()
        } else {
            // Exclude IDs already in core/recent
            let mut existing_ids = core_ids;
            existing_ids.extend(recent.iter().map(|e| e.id.clone()));

            let all_bridges = self
                .query_bridge_blooms(ctx, remaining * 2, &anchor_ids)
                .await?;
            take_included(
                all_bridges
                    .into_iter()
                    .filter(|e| !existing_ids.contains(&e.id)),
                remaining,
                include_excluded,
                &mut dropped,
            )
        };

        Ok(crate::store::WakeCascade {
            core,
            recent,
            bridges,
            excluded: tally_excluded(&dropped),
        })
    }

    /// Query all blooms with resonance >= threshold (for --min-resonance flag).
    ///
    /// Shares the core query's ordering so the sequence is stable across wakes
    /// and `wake_order` decides which bloom opens the ritual.
    ///
    /// The `?? 999999` sentinel that every cascade layer uses to sort unset
    /// orders last would collide with a legitimately stored `wake_order` of
    /// 999999. Known and not addressed here.
    async fn query_blooms_by_resonance(
        &self,
        ctx: &crate::store::AgentContext,
        threshold: i32,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE resonance >= $threshold
                AND (resonance_type IS NONE OR resonance_type != 'ephemeral')
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC,
                id ASC",
            Self::knowledge_select_fields(Projection::Lean),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("threshold", threshold));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query blooms by resonance")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 1: Query core blooms (resonance 8+, excludes ephemeral).
    ///
    /// `limit` is the SQL window, which the caller widens by the number of
    /// excluded entries it has seen so exclusion does not cost a kept entry
    /// its slot.
    async fn query_core_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE resonance >= 8
                AND (resonance_type IS NONE OR resonance_type != 'ephemeral')
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC,
                id ASC
            LIMIT $limit",
            Self::knowledge_select_fields(Projection::Lean),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query core blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 2: Query recent blooms (last N days, sorted by resonance)
    async fn query_recent_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        days: i64,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Calculate cutoff date (N days ago)
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        let cutoff_str = cutoff.to_rfc3339();

        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE last_activated > <datetime>$cutoff
                AND (resonance_type IS NONE OR resonance_type != 'ephemeral')
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC,
                id ASC
            LIMIT $limit",
            Self::knowledge_select_fields(Projection::Lean),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db
                .query(&sql)
                .bind(("cutoff", cutoff_str))
                .bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query recent blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 3: Query bridge blooms (anchored to core/recent, resonance 5+)
    async fn query_bridge_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        anchor_ids: &[String],
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Use array::intersect to check if anchors array has any overlap with anchor_ids
        // If intersection is non-empty, this bloom is anchored to a core/recent bloom
        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE array::len(array::intersect(anchors, $anchor_ids)) > 0
                AND resonance >= 5
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC,
                id ASC
            LIMIT $limit",
            Self::knowledge_select_fields(Projection::Lean),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db
                .query(&sql)
                .bind(("anchor_ids", anchor_ids.to_vec()))
                .bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query bridge blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Update activation counts for loaded blooms, resetting last_activated timestamp.
    /// Use this for intentional single-entry access (e.g. `show`, `fact-session`).
    pub fn update_activations(&self, ids: &[String]) -> Result<()> {
        Self::runtime().block_on(self.update_activations_async(ids))
    }

    async fn update_activations_async(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        // Strip "kn-" prefix from IDs if present
        let clean_ids: Vec<String> = ids
            .iter()
            .map(|id| id.strip_prefix("kn-").unwrap_or(id).to_string())
            .collect();

        // Single-id fast path: direct-record form avoids the full-table
        // pass that `WHERE id IN $ids` costs to update one row (precedent:
        // update_wake_session_async's `UPDATE type::thing(...)`).
        //
        // type::thing('knowledge', '') errors ("not a valid id") rather than
        // matching zero rows -- same hazard as get_knowledge_async's guard.
        // An empty single id (e.g. ids == [""], or ["kn-"] after the strip
        // above) must not reach type::thing; preserve the old no-op Ok(())
        // contract instead -- pinned by
        // test_update_activations_single_empty_id_is_noop in tests.rs.
        // Separately, a real (non-empty) id with no matching row is itself a
        // no-op under this path: `UPDATE type::thing(...)` against a miss
        // does not error and does not create a phantom row, with or without
        // schema applied -- covered by the miss-case assertion in
        // test_update_activations_all_digit_id_single_and_multi_path.
        if let [single_id] = clean_ids.as_slice() {
            if single_id.is_empty() {
                return Ok(());
            }

            let mut response = with_db!(self, db, {
                db.query(
                    "UPDATE type::thing('knowledge', $id) SET
                    activation_count += 1,
                    last_activated = time::now()",
                )
                .bind(("id", single_id.clone()))
                .await
                .context("Failed to update activations")
            })?;

            let errors = response.take_errors();
            if !errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "Failed to update activations: {:?}",
                    errors
                ));
            }

            return Ok(());
        }

        // Build array of Thing references
        let things: Vec<Thing> = clean_ids
            .iter()
            .map(|id| Thing::from(("knowledge", id.as_str())))
            .collect();

        let mut response = with_db!(self, db, {
            db.query(
                "UPDATE knowledge SET
                activation_count += 1,
                last_activated = time::now()
                WHERE id IN $ids",
            )
            .bind(("ids", things))
            .await
            .context("Failed to update activations")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to update activations: {:?}",
                errors
            ));
        }

        Ok(())
    }

    /// Update only the summary field of a knowledge entry.
    /// Respects visibility: agents can only update summaries on entries they can see.
    /// Returns Ok(false) for entries that don't exist OR that the agent can't see
    /// (to avoid leaking existence of private entries).
    pub fn update_summary(
        &self,
        id: &str,
        summary: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<bool> {
        Self::runtime().block_on(self.update_summary_async(id, summary, ctx))
    }

    async fn update_summary_async(
        &self,
        id: &str,
        summary: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<bool> {
        // Delegate to the builder's targeted-update path (Issue #134). This is the
        // primary place per-id knowledge `UPDATE ... SET` statements are built.
        // Three tracked exceptions hand-write their own `UPDATE knowledge SET ...`
        // because their shapes can't ride a per-id builder spec:
        // `update_activations_async` (a single-id `type::thing(...)` fast path,
        // falling back to bulk multi-id `WHERE id IN $ids` for more than one id),
        // `increment_activation_count_async` (bulk multi-id `WHERE id IN $ids`
        // only), and `reinforce_async` (read-compute-write returning a
        // ReinforcementResult).
        let spec = crate::store_update::UpdateSpec {
            fields: vec![crate::store_update::FieldUpdate {
                assignment: "summary = $set_summary".to_string(),
                param: "set_summary".to_string(),
                value: crate::store_update::UpdateValue::Str(summary.to_string()),
            }],
            add_tags: Vec::new(),
        };
        Ok(self.apply_update_async(id, &spec, ctx).await?.applied)
    }

    /// Builder-pattern targeted update (Issue #134).
    ///
    /// Runs ONE `UPDATE knowledge SET <only the columns in `spec`> WHERE ...`,
    /// binding every value (no string interpolation of caller data), under the
    /// agent visibility filter. The `SET` body is assembled solely from
    /// `spec.fields` (see [`crate::store_update::UpdateSpec::set_clause_columns`]),
    /// so a column the caller never set cannot appear in the statement — this is
    /// the no-full-record-overwrite safety property from PR #131, made structural.
    ///
    /// Any `add_tags` are applied as `tagged_with` `RELATE` edges after the
    /// column update (tags are a graph relation, not a `knowledge` column).
    pub fn apply_update(
        &self,
        id: &str,
        spec: &crate::store_update::UpdateSpec,
        ctx: &crate::store::AgentContext,
    ) -> Result<crate::store_update::UpdateOutcome> {
        Self::runtime().block_on(self.apply_update_async(id, spec, ctx))
    }

    pub(super) async fn apply_update_async(
        &self,
        id: &str,
        spec: &crate::store_update::UpdateSpec,
        ctx: &crate::store::AgentContext,
    ) -> Result<crate::store_update::UpdateOutcome> {
        use crate::store_update::{UpdateOutcome, UpdateValue};

        // Empty spec => no-op (no query). The builder guards this too, but a
        // backend caller could hand us an empty spec directly.
        if spec.is_empty() {
            return Ok(UpdateOutcome::no_op());
        }

        let id_part = id.strip_prefix("kn-").unwrap_or(id);
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Existence + visibility check. If the entry exists but isn't visible we
        // return applied=false (same as "not found") to avoid leaking the
        // existence of private entries.
        let check_sql = format!(
            "SELECT count() AS c FROM knowledge WHERE meta::id(id) = $id {} GROUP ALL",
            visibility_clause
        );

        let mut check_response = with_db!(self, db, {
            let mut query = db.query(&check_sql).bind(("id", id_part.to_string()));
            if let Some(ref agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query
                .await
                .context("Failed to check knowledge record existence for update")
        })?;

        let count_results: Vec<serde_json::Value> = check_response.take(0)?;
        let exists = count_results
            .first()
            .and_then(|v| v["c"].as_i64())
            .unwrap_or(0)
            > 0;

        if !exists {
            return Ok(UpdateOutcome::ran(false));
        }

        // Targeted column update. The SET body is built ONLY from spec.fields, so
        // unset columns are structurally absent. Re-apply the visibility filter on
        // the UPDATE itself to close the TOCTOU window between check and update.
        //
        // We always bump `updated_at = time::now()` on a column write (server-side
        // expr, binds nothing — consistent with `reinforce` and how
        // `last_activated` is handled). A record whose summary/resonance changed
        // but whose `updated_at` is stale would be misleading. A tag-only update
        // does NOT bump `updated_at`: tags are a graph edge, not a `knowledge`
        // column, so the row itself is unchanged and `updated_at` should reflect
        // column mutations only (matching `reinforce`, which only touches it on
        // its own column write).
        if spec.has_column_updates() {
            let set_body = spec.set_clause_columns();
            let update_sql = format!(
                "UPDATE knowledge SET {}, updated_at = time::now() WHERE meta::id(id) = $id {}",
                set_body, visibility_clause
            );

            let mut response = with_db!(self, db, {
                let mut query = db.query(&update_sql).bind(("id", id_part.to_string()));
                if let Some(ref agent) = current_agent {
                    query = query.bind(("current_agent", agent.clone()));
                }
                // Bind each set field's value by its param name. Fields with an
                // empty param (server-side expressions like time::now()) bind
                // nothing.
                for field in &spec.fields {
                    if field.param.is_empty() {
                        continue;
                    }
                    query = match &field.value {
                        UpdateValue::Str(s) => query.bind((field.param.clone(), s.clone())),
                        UpdateValue::Int(i) => query.bind((field.param.clone(), *i)),
                        UpdateValue::None => query,
                    };
                }
                query.await.context("Failed to apply targeted update")
            })?;

            let errors = response.take_errors();
            if !errors.is_empty() {
                return Err(anyhow::anyhow!("Failed to apply update: {:?}", errors));
            }
        }

        // Apply tag adds as tagged_with edges (graph relation, not a SET column).
        // The RELATE itself is gated on a visibility-filtered existence subquery
        // (same filter as the column UPDATE), so the edge write is subject to the
        // same TOCTOU-safe visibility check as the column path — see below.
        for tag_name in &spec.add_tags {
            self.add_tag_edge_async(id_part, tag_name, &visibility_clause, &current_agent)
                .await?;
        }

        Ok(UpdateOutcome::ran(true))
    }

    /// Ensure a tag exists and relate the knowledge entry to it (idempotent edge).
    /// Shared tag-edge logic for the builder's `add_tag`.
    ///
    /// The RELATE is folded behind a visibility-filtered existence subquery so the
    /// edge write obeys the *same* visibility filter the column UPDATE re-applies
    /// (see `apply_update_async`). This closes the TOCTOU window between the
    /// existence check and the edge write: if the entry stopped being visible to
    /// the agent in between, the subquery returns empty and the RELATE never runs,
    /// mirroring the column path's `WHERE ... <visibility_clause>` guard.
    async fn add_tag_edge_async(
        &self,
        id_part: &str,
        tag_name: &str,
        visibility_clause: &str,
        current_agent: &Option<String>,
    ) -> Result<()> {
        let knowledge = Thing::from(("knowledge", id_part));
        let tag = Thing::from(("tag", tag_name));

        // Ensure the tag node exists (matches upsert_knowledge's tag handling).
        let mut tag_response = with_db!(self, db, {
            db.query("UPSERT type::thing('tag', $tag_id) SET name = $tag_name")
                .bind(("tag_id", tag_name.to_string()))
                .bind(("tag_name", tag_name.to_string()))
                .await
                .context("Failed to create tag for update")
        })?;
        let tag_errors = tag_response.take_errors();
        if !tag_errors.is_empty() {
            return Err(anyhow::anyhow!("Failed to create tag: {:?}", tag_errors));
        }

        // Gate the RELATE on a visibility-filtered existence subquery (TOCTOU-safe,
        // symmetric with the column UPDATE) AND on the absence of an existing edge
        // (idempotent — calling add_tag twice doesn't create duplicate edges).
        // All values bound; the visibility clause references $current_agent.
        let edge_sql = format!(
            "IF (SELECT VALUE id FROM knowledge WHERE meta::id(id) = $id {visibility_clause}) != [] \
             AND (SELECT VALUE id FROM tagged_with WHERE in = $knowledge AND out = $tag) = [] \
             THEN (RELATE $knowledge->tagged_with->$tag) END"
        );
        let mut edge_response = with_db!(self, db, {
            let mut query = db
                .query(&edge_sql)
                .bind(("id", id_part.to_string()))
                .bind(("knowledge", knowledge.clone()))
                .bind(("tag", tag.clone()));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query.await.context("Failed to create tag edge for update")
        })?;
        let edge_errors = edge_response.take_errors();
        if !edge_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to create tag edge: {:?}",
                edge_errors
            ));
        }

        Ok(())
    }

    /// Increment activation_count only — does NOT reset last_activated.
    /// Use this for passive bulk surfacing (wake cascade, for-session view) where
    /// the entries were not intentionally accessed and should continue decaying
    /// at their normal rate.
    pub fn increment_activation_count(&self, ids: &[String]) -> Result<()> {
        Self::runtime().block_on(self.increment_activation_count_async(ids))
    }

    async fn increment_activation_count_async(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        // Strip "kn-" prefix from IDs if present
        let clean_ids: Vec<String> = ids
            .iter()
            .map(|id| id.strip_prefix("kn-").unwrap_or(id).to_string())
            .collect();

        // Build array of Thing references
        let things: Vec<Thing> = clean_ids
            .iter()
            .map(|id| Thing::from(("knowledge", id.as_str())))
            .collect();

        let mut response = with_db!(self, db, {
            db.query(
                "UPDATE knowledge SET
                activation_count += 1
                WHERE id IN $ids",
            )
            .bind(("ids", things))
            .await
            .context("Failed to increment activation counts")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to increment activation counts: {:?}",
                errors
            ));
        }

        Ok(())
    }

    /// Query recent ephemeral facts with decay computation
    pub fn query_recent_facts(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.query_recent_facts_async(days))
    }

    async fn query_recent_facts_async(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        // Query with computed effective_resonance for ordering and filtering.
        // Uses the shared decay formula from effective_resonance_expr().
        // This query only surfaces ephemeral entries (resonance_type = 'ephemeral');
        // foundational/transformative entries are excluded and never reach this path.
        let expr = Self::effective_resonance_expr();
        let sql = format!(
            "SELECT {},
                 ({expr}) AS effective_resonance
             FROM knowledge
             WHERE resonance_type = 'ephemeral'
             AND created_at > time::now() - duration::from::days($days)
             AND ({expr}) > 0.5
             ORDER BY effective_resonance DESC",
            Self::knowledge_select_fields(Projection::Lean),
            expr = expr
        );

        let mut response = with_db!(self, db, {
            db.query(&sql)
                .bind(("days", days))
                .await
                .context("Failed to execute recent facts query")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse recent facts results")?;

        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Query recent facts across ALL resonance types with decay computation.
    /// Foundational/transformative entries are exempt from decay (effective_resonance = resonance).
    pub fn query_recent_facts_all_types(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.query_recent_facts_all_types_async(days))
    }

    async fn query_recent_facts_all_types_async(&self, days: i32) -> Result<Vec<KnowledgeEntry>> {
        // Like query_recent_facts_async but without the resonance_type = 'ephemeral' filter.
        // Ephemeral entries are still decay-filtered (> 0.5). Foundational/transformative
        // entries are exempt from decay so they always surface here.
        let expr = Self::effective_resonance_expr();
        let sql = format!(
            "SELECT {},
                 ({expr}) AS effective_resonance
             FROM knowledge
             WHERE created_at > time::now() - duration::from::days($days)
             AND ({expr}) > 0.5
             ORDER BY effective_resonance DESC",
            Self::knowledge_select_fields(Projection::Lean),
            expr = expr
        );

        let mut response = with_db!(self, db, {
            db.query(&sql)
                .bind(("days", days))
                .await
                .context("Failed to execute recent facts (all types) query")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse recent facts (all types) results")?;

        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Reinforce a knowledge entry.
    /// Respects visibility: agents can only reinforce entries they can see.
    /// Returns Ok(None) for entries that don't exist OR that the agent can't see
    /// (to avoid leaking existence of private entries).
    pub fn reinforce(
        &self,
        id: &str,
        amount: i32,
        cap: Option<i32>,
        ctx: &crate::store::AgentContext,
    ) -> Result<Option<crate::store::ReinforcementResult>> {
        Self::runtime().block_on(self.reinforce_async(id, amount, cap, ctx))
    }

    async fn reinforce_async(
        &self,
        id: &str,
        amount: i32,
        cap: Option<i32>,
        ctx: &crate::store::AgentContext,
    ) -> Result<Option<crate::store::ReinforcementResult>> {
        // Normalize ID
        let normalized_id = if id.starts_with("kn-") {
            id.to_string()
        } else {
            format!("kn-{}", id)
        };

        let id_part = normalized_id.strip_prefix("kn-").unwrap_or(&normalized_id);

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Check if the record exists AND is visible to the current agent.
        // If the entry exists but isn't visible, we return None (same as "not found")
        // to avoid leaking the existence of private entries.
        let select_sql = format!(
            "SELECT resonance, activation_count FROM knowledge WHERE meta::id(id) = $id {}",
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&select_sql).bind(("id", id_part.to_string()));
            if let Some(ref agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query.await.context("Failed to select entry for reinforce")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse entry for reinforce")?;

        let current = match results.first() {
            Some(v) => v,
            None => return Ok(None),
        };

        let old_resonance = current
            .get("resonance")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let old_activation_count = current
            .get("activation_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        // Calculate new resonance
        let mut new_resonance = old_resonance + amount;
        let capped = if let Some(cap_value) = cap {
            if new_resonance > cap_value {
                new_resonance = cap_value;
                true
            } else {
                false
            }
        } else {
            false
        };

        let new_activation_count = old_activation_count + 1;

        // Update with the same visibility filter to prevent TOCTOU race conditions.
        // Even though we checked above, re-applying the filter on the UPDATE ensures
        // no bypass is possible between check and update.
        let update_sql = format!(
            "UPDATE knowledge SET
            resonance = $new_resonance,
            last_activated = time::now(),
            activation_count = $new_count,
            updated_at = time::now()
            WHERE meta::id(id) = $id {}",
            visibility_clause
        );

        let mut update_response = with_db!(self, db, {
            let mut query = db
                .query(&update_sql)
                .bind(("id", id_part.to_string()))
                .bind(("new_resonance", new_resonance))
                .bind(("new_count", new_activation_count));
            if let Some(ref agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query.await.context("Failed to update entry for reinforce")
        })?;

        let errors = update_response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Failed to reinforce entry: {:?}", errors));
        }

        // Get current timestamp for response
        let now = Utc::now().to_rfc3339();

        Ok(Some(crate::store::ReinforcementResult {
            id: normalized_id,
            old_resonance,
            new_resonance,
            amount_added: amount,
            capped,
            last_activated: now,
            activation_count: new_activation_count,
        }))
    }

    // =========================================================================
    // CONTENT PATCH OPERATIONS
    // =========================================================================

    /// Edit content by finding and replacing text
    pub fn edit_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        old_text: &str,
        new_text: &str,
        replace_all: bool,
        nth: Option<usize>,
    ) -> Result<crate::store::EditResult> {
        // Fetch entry
        let entry = self
            .get_knowledge(id, ctx)?
            .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

        let body = entry
            .body
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Entry has no body content"))?;

        // Use shared content operation logic
        let result = crate::content_ops::edit_content(body, old_text, new_text, replace_all, nth)?;

        // Update the entry
        let mut updated = entry;
        let content_hash = KnowledgeEntry::compute_hash(&result.new_content);
        updated.body = Some(result.new_content.clone());
        updated.updated_at = Some(chrono::Utc::now().to_rfc3339());
        updated.content_hash = Some(content_hash);

        self.upsert_knowledge_internal(&updated)?;

        Ok(crate::store::EditResult {
            replacements: result.replacements,
            new_content: result.new_content,
        })
    }

    /// Append content to the end of an entry's body
    pub fn append_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        content: &str,
    ) -> Result<()> {
        let entry = self
            .get_knowledge(id, ctx)?
            .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

        // Use shared content operation logic
        let new_body = crate::content_ops::append_content(entry.body.as_deref(), content);

        let mut updated = entry;
        let content_hash = KnowledgeEntry::compute_hash(&new_body);
        updated.body = Some(new_body);
        updated.updated_at = Some(chrono::Utc::now().to_rfc3339());
        updated.content_hash = Some(content_hash);

        self.upsert_knowledge_internal(&updated)?;
        Ok(())
    }

    /// Prepend content to the start of an entry's body
    pub fn prepend_content(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
        content: &str,
    ) -> Result<()> {
        let entry = self
            .get_knowledge(id, ctx)?
            .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

        // Use shared content operation logic
        let new_body = crate::content_ops::prepend_content(entry.body.as_deref(), content);

        let mut updated = entry;
        let content_hash = KnowledgeEntry::compute_hash(&new_body);
        updated.body = Some(new_body);
        updated.updated_at = Some(chrono::Utc::now().to_rfc3339());
        updated.content_hash = Some(content_hash);

        self.upsert_knowledge_internal(&updated)?;
        Ok(())
    }

    /// List tables - SurrealDB uses tables, return table names
    pub fn list_tables(&self) -> Result<Vec<String>> {
        Self::runtime().block_on(self.list_tables_async())
    }

    async fn list_tables_async(&self) -> Result<Vec<String>> {
        let mut response = with_db!(self, db, {
            db.query("INFO FOR DB")
                .await
                .context("Failed to query database info")
        })?;

        // SurrealDB INFO returns complex metadata - take as JSON directly
        let info: Option<serde_json::Value> = response.take(0)?;
        let mut tables = Vec::new();

        if let Some(info_json) = info
            && let Some(tables_obj) = info_json.get("tables").and_then(|v| v.as_object())
        {
            for table_name in tables_obj.keys() {
                tables.push(table_name.clone());
            }
            tables.sort();
        }

        Ok(tables)
    }

    /// Count total knowledge entries
    pub fn count(&self) -> Result<usize> {
        Self::runtime().block_on(self.count_async())
    }

    async fn count_async(&self) -> Result<usize> {
        let mut response = with_db!(self, db, {
            db.query("SELECT count() AS c FROM knowledge GROUP ALL")
                .await
                .context("Failed to count knowledge entries")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let count = results.first().and_then(|v| v["c"].as_i64()).unwrap_or(0) as usize;
        Ok(count)
    }

    /// Graph health vitality percentages.
    ///
    /// Returns a JSON object:
    ///   { "total": N, "embedded": N, "anchored": N, "stale_high_res": N,
    ///     "embedded_pct": N, "anchored_pct": N, "stale_high_res_pct": N }
    ///
    /// Counts:
    ///   embedded      — entries with a non-null embedding vector
    ///   anchored      — entries with at least one anchor relationship
    ///   stale_high_res — high-resonance entries (resonance >= 5) not activated
    ///                   in the last 30 days (potentially stale knowledge)
    pub fn graph_health(&self) -> Result<serde_json::Value> {
        Self::runtime().block_on(self.graph_health_async())
    }

    async fn graph_health_async(&self) -> Result<serde_json::Value> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT
                    count() AS total,
                    math::sum(IF embedding IS NOT NONE THEN 1 ELSE 0 END) AS embedded,
                    math::sum(IF anchors IS NOT NONE AND array::len(anchors) > 0 THEN 1 ELSE 0 END) AS anchored,
                    math::sum(IF (last_activated IS NONE OR last_activated < time::now() - duration::from::days(30)) AND resonance >= 5 THEN 1 ELSE 0 END) AS stale_high_res
                FROM knowledge GROUP ALL",
            )
            .await
            .context("Failed to query graph health")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let row = results.into_iter().next().unwrap_or_default();

        let total = row["total"].as_i64().unwrap_or(0);
        let embedded = row["embedded"].as_i64().unwrap_or(0);
        let anchored = row["anchored"].as_i64().unwrap_or(0);
        let stale_high_res = row["stale_high_res"].as_i64().unwrap_or(0);

        let pct = |n: i64| -> i64 {
            if total == 0 {
                0
            } else {
                (n * 100 + total / 2) / total
            }
        };

        Ok(serde_json::json!({
            "total": total,
            "embedded": embedded,
            "anchored": anchored,
            "stale_high_res": stale_high_res,
            "embedded_pct": pct(embedded),
            "anchored_pct": pct(anchored),
            "stale_high_res_pct": pct(stale_high_res),
        }))
    }

    /// Per-week entry counts over the last 8 weeks (oldest to newest).
    ///
    /// Returns a JSON array of up to 8 integers.  Weeks with no entries are
    /// represented as 0.  The array is always exactly 8 elements, padded with
    /// leading zeros when fewer than 8 weeks of data exist.
    pub fn growth_sparkline(&self) -> Result<serde_json::Value> {
        Self::runtime().block_on(self.growth_sparkline_async())
    }

    async fn growth_sparkline_async(&self) -> Result<serde_json::Value> {
        // Aggregated GROUP BY approach.
        // Uses the same duration syntax as the working recent-facts queries.
        // GROUP BY on the projected alias.
        let results: Vec<serde_json::Value> = {
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT
                        (<int>time::unix(<datetime>created_at) / 604800) AS week_bucket,
                        count() AS cnt
                    FROM knowledge
                    WHERE created_at > time::now() - duration::from::days(56)
                    GROUP BY week_bucket",
                )
                .await
                .context("Failed to query growth sparkline")
            })?;
            response.take(0).unwrap_or_default()
        };

        // Build a sorted map from week_bucket -> count
        let mut bucket_map: std::collections::BTreeMap<i64, i64> =
            std::collections::BTreeMap::new();
        for row in &results {
            let bucket = row["week_bucket"].as_i64().unwrap_or(0);
            let cnt = row["cnt"].as_i64().unwrap_or(0);
            bucket_map.insert(bucket, cnt);
        }

        // Fill 8 contiguous buckets ending at current week.
        // Note: dividing unix seconds by 604800 yields epoch-relative weeks
        // whose boundaries fall on Thursday 00:00 UTC (since the Unix epoch
        // was a Thursday).  The alignment is arbitrary but consistent.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let current_bucket = now_secs / 604800;

        let counts: Vec<i64> = (0i64..8)
            .map(|offset| {
                let bucket = current_bucket - (7 - offset);
                *bucket_map.get(&bucket).unwrap_or(&0)
            })
            .collect();

        Ok(serde_json::json!(counts))
    }

    /// Open threads: knowledge entries with category:thread that are not closed.
    ///
    /// Returns a JSON array sorted by decay-weighted score (resonance * 0.95^weeks_old),
    /// newest/highest-resonance first.  Each element contains the fields the dashboard
    /// thread widget needs: id, body, state, created_at, resonance, tags.
    ///
    /// Open = summary IS NONE OR summary.state IS NONE OR summary.state = "open"
    pub fn open_threads(&self) -> Result<serde_json::Value> {
        Self::runtime().block_on(self.open_threads_async())
    }

    async fn open_threads_async(&self) -> Result<serde_json::Value> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT
                    meta::id(id) AS id,
                    body,
                    summary,
                    <string>created_at AS created_at,
                    resonance,
                    ->tagged_with->tag.name AS tags
                FROM knowledge
                WHERE category = category:thread
                  AND (summary IS NONE OR summary.state IS NONE OR summary.state = 'open')
                ORDER BY created_at DESC",
            )
            .await
            .context("Failed to query open threads")
        })?;

        let rows: Vec<serde_json::Value> = response.take(0).unwrap_or_default();

        // Parse state from summary JSON; build output with stable shape
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as f64)
            .unwrap_or(0.0);

        let mut threads: Vec<serde_json::Value> = rows
            .into_iter()
            .filter_map(|row| {
                let id = row["id"].as_str().unwrap_or("").to_string();
                if id.is_empty() {
                    return None;
                }

                let summary_raw = &row["summary"];
                let state = if summary_raw.is_null()
                    || summary_raw.is_string() && summary_raw.as_str().unwrap_or("").is_empty()
                {
                    "open".to_string()
                } else {
                    let s: serde_json::Value = if let Some(s) = summary_raw.as_str() {
                        serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
                    } else {
                        summary_raw.clone()
                    };
                    s.get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("open")
                        .to_string()
                };

                // Defensive: the DB-side WHERE already filters to open threads, but
                // summary can be a raw JSON string that needs client-side parsing
                // (see the deserialisation dance above), so we re-check here in case
                // the parsed state diverges from what SurrealQL evaluated.
                if state != "open" {
                    return None;
                }

                let resonance = row["resonance"].as_i64().unwrap_or(0);
                let created_at = row["created_at"].as_str().unwrap_or("").to_string();
                let tags = row["tags"].clone();

                Some(serde_json::json!({
                    "id": format!("kn-{}", id),
                    "body": row["body"],
                    "state": state,
                    "created_at": created_at,
                    "resonance": resonance,
                    "tags": tags,
                    // Include decay score for client-side sort verification
                    "_score": Self::decay_score(resonance, &created_at, now_secs),
                }))
            })
            .collect();

        // Sort by decay-weighted score descending
        threads.sort_by(|a, b| {
            let sa = a["_score"].as_f64().unwrap_or(0.0);
            let sb = b["_score"].as_f64().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Strip the internal _score field before returning
        for t in &mut threads {
            if let Some(obj) = t.as_object_mut() {
                obj.remove("_score");
            }
        }

        Ok(serde_json::json!(threads))
    }

    /// Decay-weighted score: resonance * 0.95^weeks_old
    ///
    /// If `created_at` cannot be parsed, we treat the entry as maximally old
    /// (52 weeks) so it sinks to the bottom rather than floating to the top
    /// with zero decay.
    fn decay_score(resonance: i64, created_at: &str, now_secs: f64) -> f64 {
        let weeks = chrono::DateTime::parse_from_rfc3339(&created_at.replace('Z', "+00:00"))
            .map(|dt| {
                let created_secs = dt.timestamp() as f64;
                (now_secs - created_secs) / (7.0 * 86400.0)
            })
            .unwrap_or(52.0);

        resonance as f64 * 0.95_f64.powf(weeks)
    }

    /// List all knowledge entries
    pub fn list_all(&self, ctx: &crate::store::AgentContext) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_all_async(ctx))
    }

    async fn list_all_async(
        &self,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Convert AND to WHERE for list_all (no WHERE clause exists yet)
        let where_clause = visibility_clause.replacen("AND", "WHERE", 1);

        // ORDER BY id instead of title to avoid SurrealDB query planner
        // selecting BM25 full-text index (knowledge_title_fts) for sort
        // resolution, which crashes with "No iterator has been found".
        // See: coryzibell/mx#191
        let sql = format!(
            "SELECT {}
            FROM knowledge
            {}
            ORDER BY id",
            Self::knowledge_select_fields(Projection::Full),
            where_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql);
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query all knowledge entries")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();

        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// List entries by category (full projection, embedding included). Keep
    /// using this for `export jsonl` (Issue #438) — it's the disaster-recovery
    /// path and needs the vector to round-trip into a fresh database.
    pub fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_by_category_async(
            category,
            ctx,
            filter,
            Projection::Full,
        ))
    }

    /// Lean variant of [`list_by_category`](Self::list_by_category) (Issue
    /// #438): same rows, `embedding` dropped from the projection.
    pub fn list_by_category_lean(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_by_category_async(
            category,
            ctx,
            filter,
            Projection::Lean,
        ))
    }

    /// Fast count of entries in a category with the same visibility / resonance
    /// filtering as list_by_category, but returning only the integer count —
    /// no row hydration, no tag/applicability follow-up queries. Used by
    /// `mx memory stats` so it doesn't round-trip thousands of times per call
    /// when the db is remote.
    pub fn count_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<usize> {
        Self::runtime().block_on(self.count_by_category_async(category, ctx, filter))
    }

    async fn count_by_category_async(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<usize> {
        let category_thing = Thing::from(("category", category));
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);

        // NOTE: `SELECT count() FROM knowledge WHERE ... GROUP ALL` returns
        // the wrong number in SurrealDB 2.6 when a WHERE clause is present
        // (observed on 2.6.1: bloom with visibility='public' reports 986
        // instead of 260 — off by ~3-4x, seemingly counting some join
        // product). Wrapping the filter in a subquery that projects id only
        // gives the correct count and still avoids row hydration.
        let sql = format!(
            "SELECT count() AS c FROM (
                SELECT id FROM knowledge
                WHERE category = $category {} {}
            ) GROUP ALL",
            visibility_clause, resonance_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("category", category_thing));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to count knowledge by category")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let count = results.first().and_then(|v| v["c"].as_i64()).unwrap_or(0) as usize;
        Ok(count)
    }

    async fn list_by_category_async(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        projection: Projection,
    ) -> Result<Vec<KnowledgeEntry>> {
        let category_thing = Thing::from(("category", category));

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);

        // ORDER BY id instead of title — see comment in list_all_async
        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE category = $category {} {}
            ORDER BY id",
            Self::knowledge_select_fields(projection),
            visibility_clause,
            resonance_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("category", category_thing));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query knowledge by category")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();

        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Fetch the caller's OWN private entries matching the same category /
    /// resonance / (optional) full-text filters as list/search (Issue #400).
    /// See the trait doc on `KnowledgeStore::owned_private_matching`.
    pub fn owned_private_matching(
        &self,
        agent: &str,
        query: Option<&str>,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.owned_private_matching_async(agent, query, filter))
    }

    async fn owned_private_matching_async(
        &self,
        agent: &str,
        query: Option<&str>,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        // Reuse the SAME clause builders the main list/search queries use so the
        // hint count stays consistent with what those commands would display
        // (kn-e8d7eff2): effective/decayed resonance via build_resonance_filter,
        // category matching via build_category_filter.
        let resonance_clause = Self::build_resonance_filter(filter);
        let category_clause = Self::build_category_filter(filter);

        // Visibility is FIXED to the caller's own private rows. We deliberately
        // do NOT call build_visibility_filter here: that helper also admits
        // public rows, but this query must count ONLY owned-private matches (the
        // rows the public-only default hides) and must NEVER touch another
        // agent's private entries (visibility-bypass pipe-dream kn-a5f8a209).
        // $current_agent is a bound parameter, never interpolated.
        let search_clause = if query.is_some() {
            "AND (title @@ $query OR body @@ $query OR summary @@ $query)"
        } else {
            ""
        };

        // S1 — accepted cost: this SELECTs and hydrates FULL `KnowledgeEntry`
        // rows (body + per-row tag/applicability follow-ups in
        // `value_to_knowledge_entry`), not a bare COUNT, and it runs on every
        // default `list`/`search` when a calling agent is set. Full hydration is
        // REQUIRED, not incidental: the hint count must match the main query
        // exactly, and the caller re-applies `apply_entry_filters` (tags + field
        // presence — e.g. has_anchors, has_wake_phrase) to these rows. Those
        // predicates read fields a COUNT could not project, so a COUNT here would
        // over-count relative to what the main query actually displays. The rows
        // are the caller's own private entries only (bounded, single query), so
        // the cost is bounded and deemed acceptable versus correctness.
        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE visibility = 'private' AND owner = $current_agent {} {} {}
            ORDER BY id",
            Self::knowledge_select_fields(Projection::Lean),
            search_clause,
            resonance_clause,
            category_clause
        );

        let mut response = with_db!(self, db, {
            let mut q = db.query(&sql).bind(("current_agent", agent.to_string()));
            if let Some(query_str) = query {
                q = q.bind(("query", query_str.to_string()));
            }
            q.await
                .context("Failed to query owned private matches (Issue #400 hint)")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    // =========================================================================
    // WAKE SESSION OPERATIONS
    // =========================================================================

    /// Create a wake session record, return the session_id
    pub fn create_wake_session(&self, session: &crate::wake_token::WakeSession) -> Result<String> {
        Self::runtime().block_on(self.create_wake_session_async(session))
    }

    async fn create_wake_session_async(
        &self,
        session: &crate::wake_token::WakeSession,
    ) -> Result<String> {
        // Serialize bloom_chunk_meta as a JSON array. The schema field is
        // `flexible array<object>` so SurrealDB will accept arbitrary shape.
        let bloom_chunk_meta_json = serde_json::to_value(&session.bloom_chunk_meta)?;
        let created_at = chrono::DateTime::from_timestamp(session.created_at, 0)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

        let mut response = with_db!(self, db, {
            db.query(
                "CREATE type::thing('wake_session', $session_id) SET
                    agent = $agent,
                    wake = $wake,
                    model_id = $model_id,
                    bloom_ids = $bloom_ids,
                    current_index = $current_index,
                    current_chunk_index = $current_chunk_index,
                    step = $step,
                    unhinted_count = $unhinted_count,
                    revealed_count = $revealed_count,
                    unjudged_count = $unjudged_count,
                    created_at = <datetime>$created_at,
                    bloom_chunk_meta = $bloom_chunk_meta
                ",
            )
            .bind(("session_id", session.session_id.clone()))
            .bind(("agent", session.agent.clone()))
            .bind(("wake", session.wake))
            .bind(("model_id", session.model_id.clone()))
            .bind(("bloom_ids", session.bloom_ids.clone()))
            .bind(("current_index", session.current_index as i64))
            .bind(("current_chunk_index", session.current_chunk_index as i64))
            .bind(("step", session.step as i64))
            .bind(("unhinted_count", session.unhinted_count as i64))
            .bind(("revealed_count", session.revealed_count as i64))
            .bind(("unjudged_count", session.unjudged_count as i64))
            .bind(("created_at", normalize_datetime(&created_at)))
            .bind(("bloom_chunk_meta", bloom_chunk_meta_json))
            .await
            .context("Failed to create wake session")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB error creating wake session: {:?}",
                errors
            ));
        }

        Ok(session.session_id.clone())
    }

    /// Get a wake session by ID
    pub fn get_wake_session(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::wake_token::WakeSession>> {
        Self::runtime().block_on(self.get_wake_session_async(session_id))
    }

    async fn get_wake_session_async(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::wake_token::WakeSession>> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT
                    meta::id(id) AS session_id,
                    agent,
                    wake,
                    model_id,
                    bloom_ids,
                    current_index,
                    current_chunk_index,
                    step,
                    unhinted_count,
                    revealed_count,
                    unjudged_count,
                    <int>time::unix(<datetime>created_at) AS created_at,
                    IF completed_at != NONE THEN <int>time::unix(completed_at) END AS completed_at,
                    prev_step,
                    last_status,
                    bloom_chunk_meta
                FROM type::thing('wake_session', $session_id)",
            )
            .bind(("session_id", session_id.to_string()))
            .await
            .context("Failed to get wake session")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        if results.is_empty() {
            return Ok(None);
        }

        let obj = &results[0];

        let session_id_str = obj["session_id"].as_str().unwrap_or_default().to_string();
        let bloom_ids: Vec<String> = obj["bloom_ids"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let current_index = obj["current_index"].as_u64().unwrap_or(0) as usize;
        // Diffi flagged the previous `as u64 as u16` pattern as a silent-wrap
        // footgun — reaching u16::MAX requires ~2000x default-threshold chunks
        // today but the cast hides the failure mode. `try_from` surfaces an
        // out-of-range stored value as a deserialization error instead of
        // quietly producing a wrong cursor value.
        let raw_chunk_idx = obj["current_chunk_index"].as_u64().unwrap_or(0);
        let current_chunk_index = u16::try_from(raw_chunk_idx).map_err(|_| {
            anyhow::anyhow!(
                "wake_session.current_chunk_index {} exceeds u16::MAX; \
                 session is corrupt or schema has drifted",
                raw_chunk_idx
            )
        })?;
        // A session row written before the one-guess ritual has no `agent`
        // field at all. Every other new field defaults harmlessly, so without
        // this check the old session would walk to completion and file every
        // guess under an empty agent — unreachable from a log keyed by agent.
        // Absent, not merely empty: the field is required on every row this
        // binary writes.
        let agent = match obj.get("agent").and_then(|v| v.as_str()) {
            Some(agent) => agent.to_string(),
            None => bail!(
                "Wake session {} was created by an older version of mx and cannot be \
                 continued. Run `mx memory wake --begin` to start a new ritual.",
                session_id
            ),
        };

        let step = obj["step"].as_u64().unwrap_or(0) as u32;
        let unhinted_count = obj["unhinted_count"].as_u64().unwrap_or(0) as u32;
        let revealed_count = obj["revealed_count"].as_u64().unwrap_or(0) as u32;
        let unjudged_count = obj["unjudged_count"].as_u64().unwrap_or(0) as u32;
        let wake = obj["wake"].as_i64();
        let model_id = obj
            .get("model_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let created_at = obj["created_at"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        // Deserialize bloom_chunk_meta via serde_json. A session written by an
        // older binary has a different shape and deserializes to nothing; a
        // length mismatch means the same. Either way the outcome counters
        // restart at zero rather than blocking the walk.
        let bloom_chunk_meta: Vec<crate::wake_token::BloomChunkMeta> =
            match obj.get("bloom_chunk_meta") {
                Some(v) if !v.is_null() => serde_json::from_value(v.clone()).unwrap_or_default(),
                _ => Vec::new(),
            };
        let bloom_chunk_meta = if bloom_chunk_meta.len() == bloom_ids.len() {
            bloom_chunk_meta
        } else {
            vec![crate::wake_token::BloomChunkMeta::default(); bloom_ids.len()]
        };

        Ok(Some(crate::wake_token::WakeSession {
            session_id: session_id_str,
            agent,
            wake,
            model_id,
            bloom_ids,
            current_index,
            current_chunk_index,
            step,
            unhinted_count,
            revealed_count,
            unjudged_count,
            created_at,
            completed_at: obj["completed_at"].as_i64(),
            prev_step: obj["prev_step"].as_u64().map(u32::try_from).transpose()?,
            last_status: obj["last_status"].as_str().map(str::to_string),
            bloom_chunk_meta,
        }))
    }

    /// Save a mutated session as a compare-and-swap on `step`.
    pub fn update_wake_session(
        &self,
        session: &crate::wake_token::WakeSession,
        expected_step: u32,
    ) -> Result<()> {
        Self::runtime().block_on(self.update_wake_session_async(session, expected_step))
    }

    async fn update_wake_session_async(
        &self,
        session: &crate::wake_token::WakeSession,
        expected_step: u32,
    ) -> Result<()> {
        let fields = wake_session_fields(session)?;
        let sql = format!(
            "{WAKE_SESSION_CAS}
            IF array::len($moved) = 0 {{ THROW $moved_error }};"
        );

        let mut response = with_db!(self, db, {
            db.query(sql)
                .bind(("session_id", session.session_id.clone()))
                .bind(("s", fields))
                .bind(("complete", session.is_complete()))
                .bind(("expected_step", expected_step as i64))
                .bind(("moved_error", wake_session_moved_error(expected_step)))
                .await
                .context("Failed to update wake session")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB error updating wake session: {:?}",
                errors
            ));
        }

        Ok(())
    }

    // =========================================================================
    // WAKE GUESS LOG
    // =========================================================================

    /// Log one guess and advance its session in one transaction.
    ///
    /// The row id is `wake_guess:[session_id, position]`, so a second write for
    /// the same step collides on the id as well as on the `wake_guess_step`
    /// index (which also catches rows older binaries wrote under random ids).
    /// The session write is a compare-and-swap on `step`. If either fails the
    /// transaction is cancelled and neither lands.
    ///
    /// The scoring columns (`embedding`, `sim_*`, `scored_at`) are left unset:
    /// a row with a null `scored_at` is pending.
    pub fn record_wake_guess(
        &self,
        row: &crate::wake_guess::WakeGuessRow,
        session: &crate::wake_token::WakeSession,
        expected_step: u32,
    ) -> Result<()> {
        Self::runtime().block_on(self.record_wake_guess_async(row, session, expected_step))
    }

    async fn record_wake_guess_async(
        &self,
        row: &crate::wake_guess::WakeGuessRow,
        session: &crate::wake_token::WakeSession,
        expected_step: u32,
    ) -> Result<()> {
        let fields = wake_session_fields(session)?;
        let sql = format!(
            "BEGIN TRANSACTION;
            CREATE type::thing('wake_guess', [$row_session_id, $position]) SET
                agent = $agent,
                wake = $wake,
                session_id = $row_session_id,
                bloom_id = $bloom_id,
                chunk_index = $chunk_index,
                chunk_total = $chunk_total,
                position = $position,
                bloom_position = $bloom_position,
                bloom_total = $bloom_total,
                title_shown = $title_shown,
                guess = $guess,
                model_id = $model_id,
                phrase_source = $phrase_source,
                phrases = $phrases,
                match_kind = $match_kind,
                match_index = $match_index,
                bucket = $bucket,
                content_hash = $content_hash
            RETURN NONE;
            {WAKE_SESSION_CAS}
            IF array::len($moved) = 0 {{ THROW $moved_error }};
            COMMIT TRANSACTION;"
        );

        let mut response = with_db!(self, db, {
            db.query(sql)
                .bind(("agent", row.agent.clone()))
                .bind(("wake", row.wake))
                .bind(("row_session_id", row.session_id.clone()))
                .bind(("bloom_id", row.bloom_id.clone()))
                .bind(("chunk_index", row.chunk_index as i64))
                .bind(("chunk_total", row.chunk_total as i64))
                .bind(("position", row.position as i64))
                .bind(("bloom_position", row.bloom_position as i64))
                .bind(("bloom_total", row.bloom_total as i64))
                .bind(("title_shown", row.title_shown.clone()))
                .bind(("guess", row.guess.clone()))
                .bind(("model_id", row.model_id.clone()))
                .bind(("phrase_source", row.phrase_source.clone()))
                .bind(("phrases", row.phrases.clone()))
                .bind(("match_kind", row.match_kind.clone()))
                .bind(("match_index", row.match_index.map(|i| i as i64)))
                .bind(("bucket", row.bucket.clone()))
                .bind(("content_hash", row.content_hash.clone()))
                .bind(("session_id", session.session_id.clone()))
                .bind(("s", fields))
                .bind(("complete", session.is_complete()))
                .bind(("expected_step", expected_step as i64))
                .bind(("moved_error", wake_session_moved_error(expected_step)))
                .await
                .context("Failed to write wake guess row")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB error writing wake guess row: {:?}",
                errors
            ));
        }

        Ok(())
    }

    /// The guess logged at one step of a session. Looked up by the
    /// `(session_id, position)` pair rather than by record id, so a row an
    /// older binary wrote under a random id is found too.
    pub fn get_wake_guess(
        &self,
        session_id: &str,
        position: u32,
    ) -> Result<Option<crate::wake_guess::WakeGuessRow>> {
        Self::runtime().block_on(self.get_wake_guess_async(session_id, position))
    }

    async fn get_wake_guess_async(
        &self,
        session_id: &str,
        position: u32,
    ) -> Result<Option<crate::wake_guess::WakeGuessRow>> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT agent, wake, session_id, bloom_id, chunk_index, chunk_total,
                    position, bloom_position, bloom_total, title_shown, guess, model_id,
                    phrase_source, phrases, match_kind, match_index, bucket, content_hash
                FROM wake_guess
                WHERE session_id = $session_id AND position = $position
                LIMIT 1",
            )
            .bind(("session_id", session_id.to_string()))
            .bind(("position", position as i64))
            .await
            .context("Failed to read wake guess row")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let Some(obj) = results.first() else {
            return Ok(None);
        };

        let int = |key: &str| obj[key].as_i64().unwrap_or(0);
        let text = |key: &str| obj[key].as_str().unwrap_or_default().to_string();
        Ok(Some(crate::wake_guess::WakeGuessRow {
            agent: text("agent"),
            wake: obj["wake"].as_i64(),
            session_id: text("session_id"),
            bloom_id: text("bloom_id"),
            chunk_index: u16::try_from(int("chunk_index"))?,
            chunk_total: u16::try_from(int("chunk_total"))?,
            position: u32::try_from(int("position"))?,
            bloom_position: usize::try_from(int("bloom_position"))?,
            bloom_total: usize::try_from(int("bloom_total"))?,
            title_shown: text("title_shown"),
            guess: text("guess"),
            model_id: obj["model_id"].as_str().map(str::to_string),
            phrase_source: text("phrase_source"),
            phrases: obj["phrases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            match_kind: text("match_kind"),
            match_index: obj["match_index"]
                .as_i64()
                .map(usize::try_from)
                .transpose()?,
            bucket: text("bucket"),
            content_hash: text("content_hash"),
        }))
    }

    /// Which sessions already logged rows under `wake` for `agent`, and the
    /// highest wake number the agent has logged. Both read the
    /// `wake_guess_wake` index.
    pub fn wake_history(&self, agent: &str, wake: i64) -> Result<crate::wake_guess::WakeHistory> {
        Self::runtime().block_on(self.wake_history_async(agent, wake))
    }

    async fn wake_history_async(
        &self,
        agent: &str,
        wake: i64,
    ) -> Result<crate::wake_guess::WakeHistory> {
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT session_id FROM wake_guess
                    WHERE agent = $agent AND wake = $wake GROUP BY session_id;
                SELECT wake FROM wake_guess
                    WHERE agent = $agent AND wake != NONE ORDER BY wake DESC LIMIT 1;",
            )
            .bind(("agent", agent.to_string()))
            .bind(("wake", wake))
            .await
            .context("Failed to read wake guess history")
        })?;

        let sessions: Vec<serde_json::Value> = response.take(0)?;
        let highest: Vec<serde_json::Value> = response.take(1)?;
        Ok(crate::wake_guess::WakeHistory {
            sessions_at_wake: sessions
                .iter()
                .filter_map(|v| v["session_id"].as_str().map(str::to_string))
                .collect(),
            highest_wake: highest.first().and_then(|v| v["wake"].as_i64()),
        })
    }

    /// Run a raw read query and return the rows as JSON.
    ///
    /// Test-only: the tables without a typed reader (currently `wake_guess`,
    /// whose reader lands with the scoring commands) still need their
    /// SCHEMAFULL definitions pinned by a round-trip test.
    #[cfg(test)]
    pub(crate) fn query_json_for_test(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        Self::runtime().block_on(self.query_json_for_test_async(sql))
    }

    #[cfg(test)]
    async fn query_json_for_test_async(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let mut response = with_db!(self, db, {
            db.query(sql).await.context("Failed to run test query")
        })?;
        Ok(response.take(0)?)
    }

    // =========================================================================
    // GHOST ANCHOR SWEEP
    // =========================================================================

    /// Sweep anchor fields for references to deleted/missing entries.
    ///
    /// Strategy:
    ///   1. Fetch all entries that have at least one anchor (no visibility
    ///      filter — soren-vault has full access and we must repair ALL entries).
    ///   2. Collect every unique anchor ID referenced across the graph.
    ///   3. Batch-check which of those IDs actually exist in the knowledge table.
    ///   4. For each entry, compute the set of ghost anchors (referenced but
    ///      missing from the existence set).
    ///   5. If dry_run=false, UPDATE each affected entry to remove ghosts.
    ///
    /// Returns a `GhostSweepResult` with full accounting.
    pub fn sweep_ghost_anchors(&self, dry_run: bool) -> Result<crate::store::GhostSweepResult> {
        Self::runtime().block_on(self.sweep_ghost_anchors_async(dry_run))
    }

    async fn sweep_ghost_anchors_async(
        &self,
        dry_run: bool,
    ) -> Result<crate::store::GhostSweepResult> {
        // ----------------------------------------------------------------
        // Phase 1: Fetch all entries with non-empty anchors.
        // No visibility filter — this is a maintenance operation running as
        // soren-vault. We need to repair all entries regardless of visibility.
        // ----------------------------------------------------------------
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT meta::id(id) AS id, title, anchors
                 FROM knowledge
                 WHERE array::len(anchors) > 0
                 ORDER BY id",
            )
            .await
            .context("Failed to query anchored entries for ghost sweep")
        })?;

        let raw: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse anchored entries")?;

        // Parse records, normalizing anchors to plain strings.
        // Anchors are stored as plain strings in SurrealDB, but may come back
        // as Thing objects ({ tb: "knowledge", id: "..." }) depending on how
        // they were originally inserted. Handle both forms.
        struct ParsedEntry {
            id: String,
            title: String,
            anchors: Vec<String>,
        }

        let mut anchored_entries: Vec<ParsedEntry> = Vec::new();
        for obj in raw {
            let id = obj["id"].as_str().unwrap_or("").to_string();
            let title = obj["title"].as_str().unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }

            let anchors_raw = obj["anchors"].as_array().cloned().unwrap_or_default();
            let anchors: Vec<String> = anchors_raw
                .into_iter()
                .filter_map(|v| {
                    // Plain string form: "kn-abc123" or "abc123"
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                    // Object form from Thing deserialization
                    if let Some(obj) = v.as_object()
                        && let Some(id_val) = obj.get("id")
                    {
                        return id_val.as_str().map(|s| s.to_string());
                    }
                    None
                })
                .collect();

            if !anchors.is_empty() {
                anchored_entries.push(ParsedEntry { id, title, anchors });
            }
        }

        let entries_scanned = anchored_entries.len();

        if entries_scanned == 0 {
            return Ok(crate::store::GhostSweepResult {
                entries_scanned: 0,
                ghosts_found: 0,
                ghosts_removed: 0,
                affected_entries: vec![],
                dry_run,
            });
        }

        // ----------------------------------------------------------------
        // Phase 2: Collect all unique anchor IDs referenced anywhere.
        // ----------------------------------------------------------------
        let mut all_referenced: HashSet<String> = HashSet::new();
        for entry in &anchored_entries {
            for anchor in &entry.anchors {
                // Normalize: strip "kn-" prefix for the existence check since
                // the knowledge table's meta::id returns the bare suffix.
                let bare = anchor.strip_prefix("kn-").unwrap_or(anchor).to_string();
                all_referenced.insert(bare);
            }
        }

        // ----------------------------------------------------------------
        // Phase 3: Batch-check existence.
        // Build Things for all referenced IDs and query which ones exist.
        // ----------------------------------------------------------------
        let referenced_vec: Vec<String> = all_referenced.into_iter().collect();
        let things: Vec<Thing> = referenced_vec
            .iter()
            .map(|id| Thing::from(("knowledge", id.as_str())))
            .collect();

        let mut exist_response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id FROM knowledge WHERE id IN $ids")
                .bind(("ids", things))
                .await
                .context("Failed to check anchor target existence")
        })?;

        let exist_raw: Vec<serde_json::Value> = exist_response
            .take(0)
            .context("Failed to parse existence results")?;

        // Build the live-ID set (bare IDs without "kn-" prefix).
        let live_ids: HashSet<String> = exist_raw
            .into_iter()
            .filter_map(|v| v["id"].as_str().map(|s| s.to_string()))
            .collect();

        // ----------------------------------------------------------------
        // Phase 4: Find ghost anchors per entry.
        // Uses the extracted `detect_ghosts` pure function for testability.
        // ----------------------------------------------------------------
        let mut affected_entries: Vec<crate::store::GhostEntry> = Vec::new();
        let mut total_ghosts = 0usize;

        for entry in &anchored_entries {
            let ghost_anchors = detect_ghosts(&entry.anchors, &live_ids);

            if !ghost_anchors.is_empty() {
                total_ghosts += ghost_anchors.len();
                affected_entries.push(crate::store::GhostEntry {
                    id: format!("kn-{}", entry.id),
                    title: entry.title.clone(),
                    ghost_anchors,
                });
            }
        }

        // ----------------------------------------------------------------
        // Phase 5: Remove ghost anchors (unless dry run).
        //
        // Instead of snapshotting the full anchor list and overwriting it,
        // we subtract ghosts using array::complement inside the UPDATE.
        // This eliminates the TOCTOU window: if another process adds a new
        // anchor between Phases 1-4 and this write, that anchor is never in
        // $ghost_ids and therefore survives untouched.
        //
        // All ghosts for a single entry are batched into one query:
        //   UPDATE knowledge
        //   SET anchors = array::complement(anchors, $ghost_ids),
        //       updated_at = time::now()
        //   WHERE meta::id(id) = $entry_id
        //
        // array::complement(a, b) returns every element of `a` not present
        // in `b`, so passing the full ghost vec removes exactly those values
        // without touching anything else in the live array.
        // ----------------------------------------------------------------
        let mut ghosts_removed = 0usize;

        if !dry_run && !affected_entries.is_empty() {
            for ghost_entry in &affected_entries {
                let bare_id = ghost_entry
                    .id
                    .strip_prefix("kn-")
                    .unwrap_or(&ghost_entry.id);

                let ghost_ids = ghost_entry.ghost_anchors.clone();
                let ghost_count = ghost_ids.len();

                let mut update_response = with_db!(self, db, {
                    db.query(
                        "UPDATE knowledge
                         SET anchors = array::complement(anchors, $ghost_ids),
                             updated_at = time::now()
                         WHERE meta::id(id) = $entry_id",
                    )
                    .bind(("entry_id", bare_id.to_string()))
                    .bind(("ghost_ids", ghost_ids))
                    .await
                    .context("Failed to remove ghost anchors during sweep")
                })?;

                let errors = update_response.take_errors();
                if !errors.is_empty() {
                    // Non-fatal: log the failure and continue sweeping.
                    eprintln!(
                        "sweep-ghosts: failed to remove {} ghost anchor(s) from {} — {:?}",
                        ghost_count, ghost_entry.id, errors
                    );
                    continue;
                }

                ghosts_removed += ghost_count;
            }
        }

        Ok(crate::store::GhostSweepResult {
            entries_scanned,
            ghosts_found: total_ghosts,
            ghosts_removed,
            affected_entries,
            dry_run,
        })
    }
}

// =========================================================================
// GHOST DETECTION — Pure function for testability
// =========================================================================

/// Identify ghost anchors for a single entry.
///
/// An anchor is a "ghost" if its bare ID (with "kn-" prefix stripped) does not
/// appear in `live_ids`. Returns the list of ghost anchor strings (preserving
/// their original form, including any "kn-" prefix they may carry).
///
/// This is the core detection logic used by `sweep_ghost_anchors_async`, extracted
/// as a pure function so it can be tested without a database connection.
pub(crate) fn detect_ghosts(anchors: &[String], live_ids: &HashSet<String>) -> Vec<String> {
    anchors
        .iter()
        .filter(|anchor| {
            let bare = anchor.strip_prefix("kn-").unwrap_or(anchor);
            !live_ids.contains(bare)
        })
        .cloned()
        .collect()
}

/// The session write shared by `update_wake_session` and `record_wake_guess`:
/// a compare-and-swap on `step`, leaving what it updated in `$moved`.
const WAKE_SESSION_CAS: &str = "LET $moved = (UPDATE type::thing('wake_session', $session_id) SET
        current_index = $s.current_index,
        current_chunk_index = $s.current_chunk_index,
        step = $s.step,
        unhinted_count = $s.unhinted_count,
        revealed_count = $s.revealed_count,
        unjudged_count = $s.unjudged_count,
        bloom_chunk_meta = $s.bloom_chunk_meta,
        completed_at = IF $complete THEN time::now() END,
        prev_step = $expected_step,
        last_status = $s.last_status
    WHERE step = $expected_step
    RETURN AFTER);";

fn wake_session_fields(session: &crate::wake_token::WakeSession) -> Result<serde_json::Value> {
    // `last_status` is left out when unset: a JSON null would reach the
    // SCHEMAFULL `option<string>` field as NULL, which it rejects.
    let mut fields = serde_json::json!({
        "current_index": session.current_index as i64,
        "current_chunk_index": session.current_chunk_index as i64,
        "step": session.step as i64,
        "unhinted_count": session.unhinted_count as i64,
        "revealed_count": session.revealed_count as i64,
        "unjudged_count": session.unjudged_count as i64,
        "bloom_chunk_meta": serde_json::to_value(&session.bloom_chunk_meta)?,
    });
    if let Some(status) = &session.last_status {
        fields["last_status"] = serde_json::Value::String(status.clone());
    }
    Ok(fields)
}

fn wake_session_moved_error(expected_step: u32) -> String {
    format!(
        "wake session is no longer at step {}; another call advanced it",
        expected_step
    )
}
