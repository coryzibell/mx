use anyhow::{Context, Result};
use chrono::Utc;
use surrealdb::sql::Thing;

use crate::knowledge::KnowledgeEntry;

use super::connection::normalize_datetime;
use super::{SurrealConnection, SurrealDatabase};

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

    /// Wake-up cascade: Load Q's identity through three layers of resonance
    pub fn wake_cascade(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        Self::runtime().block_on(self.wake_cascade_async(ctx, limit, min_resonance, days))
    }

    async fn wake_cascade_async(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        // If min_resonance is set, use simple query for all blooms >= threshold
        if let Some(threshold) = min_resonance {
            let blooms = self.query_blooms_by_resonance(ctx, threshold).await?;
            return Ok(crate::store::WakeCascade {
                core: blooms,
                recent: Vec::new(),
                bridges: Vec::new(),
            });
        }

        // Sequential filling: core first, then recent, then bridges
        // This ensures we get the most important blooms first

        // Layer 1: Core foundational/transformative blooms (resonance 8+)
        // Use full limit for core - we'll subtract what we get
        let core = self.query_core_blooms(ctx, limit).await?;
        let remaining = limit.saturating_sub(core.len());

        // Layer 2: Recent blooms (last N days)
        // Exclude IDs already in core, use remaining quota
        let core_ids: std::collections::HashSet<String> =
            core.iter().map(|e| e.id.clone()).collect();

        let all_recent = self.query_recent_blooms(ctx, remaining * 2, days).await?;
        let recent: Vec<_> = all_recent
            .into_iter()
            .filter(|e| !core_ids.contains(&e.id))
            .take(remaining)
            .collect();
        let remaining = remaining.saturating_sub(recent.len());

        // Layer 3: Bridge blooms (anchored to core/recent, resonance 5+)
        // Use final remaining quota
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
            all_bridges
                .into_iter()
                .filter(|e| !existing_ids.contains(&e.id))
                .take(remaining)
                .collect()
        };

        Ok(crate::store::WakeCascade {
            core,
            recent,
            bridges,
        })
    }

    /// Query all blooms with resonance >= threshold (for --min-resonance flag)
    async fn query_blooms_by_resonance(
        &self,
        ctx: &crate::store::AgentContext,
        threshold: i32,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE resonance >= $threshold
            AND (resonance_type IS NONE OR resonance_type != 'ephemeral')
            {}
            ORDER BY resonance DESC",
            Self::knowledge_select_fields(),
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

    /// Layer 1: Query core blooms (resonance 8+, excludes ephemeral)
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
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
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
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
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
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
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
        let id_part = id.strip_prefix("kn-").unwrap_or(id);

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Check if the record exists AND is visible to the current agent.
        // If the entry exists but isn't visible, we return false (same as "not found")
        // to avoid leaking the existence of private entries.
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
                .context("Failed to check knowledge record existence for summary update")
        })?;

        let count_results: Vec<serde_json::Value> = check_response.take(0)?;
        let exists = count_results
            .first()
            .and_then(|v| v["c"].as_i64())
            .unwrap_or(0)
            > 0;

        if !exists {
            return Ok(false);
        }

        // Update with the same visibility filter to prevent TOCTOU race conditions.
        // Even though we checked above, re-applying the filter on the UPDATE ensures
        // no bypass is possible between check and update.
        let update_sql = format!(
            "UPDATE knowledge SET summary = $summary WHERE meta::id(id) = $id {}",
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db
                .query(&update_sql)
                .bind(("id", id_part.to_string()))
                .bind(("summary", summary.to_string()));
            if let Some(ref agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query.await.context("Failed to update summary")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Failed to update summary: {:?}", errors));
        }

        Ok(true)
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
            Self::knowledge_select_fields(),
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
            Self::knowledge_select_fields(),
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
                    GROUP BY week_bucket
                    ORDER BY week_bucket",
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
            Self::knowledge_select_fields(),
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

    /// List entries by category
    pub fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_by_category_async(category, ctx, filter))
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
            Self::knowledge_select_fields(),
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
                    bloom_ids = $bloom_ids,
                    current_index = $current_index,
                    current_chunk_index = $current_chunk_index,
                    step = $step,
                    attempts_on_current = $attempts_on_current,
                    remembered_count = $remembered_count,
                    needed_help_count = $needed_help_count,
                    skipped_count = $skipped_count,
                    created_at = <datetime>$created_at,
                    bloom_chunk_meta = $bloom_chunk_meta
                ",
            )
            .bind(("session_id", session.session_id.clone()))
            .bind(("bloom_ids", session.bloom_ids.clone()))
            .bind(("current_index", session.current_index as i64))
            .bind(("current_chunk_index", session.current_chunk_index as i64))
            .bind(("step", session.step as i64))
            .bind(("attempts_on_current", session.attempts_on_current as i64))
            .bind(("remembered_count", session.remembered_count as i64))
            .bind(("needed_help_count", session.needed_help_count as i64))
            .bind(("skipped_count", session.skipped_count as i64))
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
                    bloom_ids,
                    current_index,
                    current_chunk_index,
                    step,
                    attempts_on_current,
                    remembered_count,
                    needed_help_count,
                    skipped_count,
                    <int>time::unix(<datetime>created_at) AS created_at,
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
        let step = obj["step"].as_u64().unwrap_or(0) as u32;
        let attempts_on_current = obj["attempts_on_current"].as_u64().unwrap_or(0) as u8;
        let remembered_count = obj["remembered_count"].as_u64().unwrap_or(0) as u32;
        let needed_help_count = obj["needed_help_count"].as_u64().unwrap_or(0) as u32;
        let skipped_count = obj["skipped_count"].as_u64().unwrap_or(0) as u32;
        let created_at = obj["created_at"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        // Deserialize bloom_chunk_meta via serde_json. Absent/empty → default
        // to one meta per bloom_id marking every bloom as phraseless (safe
        // fallback that keeps the session walkable via skips).
        let bloom_chunk_meta: Vec<crate::wake_token::BloomChunkMeta> =
            match obj.get("bloom_chunk_meta") {
                Some(v) if !v.is_null() => serde_json::from_value(v.clone()).unwrap_or_default(),
                _ => Vec::new(),
            };
        let bloom_chunk_meta = if bloom_chunk_meta.len() == bloom_ids.len() {
            bloom_chunk_meta
        } else {
            // Length mismatch — rebuild default metadata so the session can
            // at least walk (all phraseless, will drop through skip path).
            bloom_ids
                .iter()
                .map(|_| crate::wake_token::BloomChunkMeta {
                    authored_phrase_count: 0,
                    is_phraseless: true,
                    ..Default::default()
                })
                .collect()
        };

        Ok(Some(crate::wake_token::WakeSession {
            session_id: session_id_str,
            bloom_ids,
            current_index,
            current_chunk_index,
            step,
            attempts_on_current,
            remembered_count,
            needed_help_count,
            skipped_count,
            created_at,
            bloom_chunk_meta,
        }))
    }

    /// Update an existing wake session
    pub fn update_wake_session(&self, session: &crate::wake_token::WakeSession) -> Result<()> {
        Self::runtime().block_on(self.update_wake_session_async(session))
    }

    async fn update_wake_session_async(
        &self,
        session: &crate::wake_token::WakeSession,
    ) -> Result<()> {
        let bloom_chunk_meta_json = serde_json::to_value(&session.bloom_chunk_meta)?;

        let mut response = with_db!(self, db, {
            db.query(
                "UPDATE type::thing('wake_session', $session_id) SET
                    current_index = $current_index,
                    current_chunk_index = $current_chunk_index,
                    step = $step,
                    attempts_on_current = $attempts_on_current,
                    remembered_count = $remembered_count,
                    needed_help_count = $needed_help_count,
                    skipped_count = $skipped_count,
                    bloom_chunk_meta = $bloom_chunk_meta
                ",
            )
            .bind(("session_id", session.session_id.clone()))
            .bind(("current_index", session.current_index as i64))
            .bind(("current_chunk_index", session.current_chunk_index as i64))
            .bind(("step", session.step as i64))
            .bind(("attempts_on_current", session.attempts_on_current as i64))
            .bind(("remembered_count", session.remembered_count as i64))
            .bind(("needed_help_count", session.needed_help_count as i64))
            .bind(("skipped_count", session.skipped_count as i64))
            .bind(("bloom_chunk_meta", bloom_chunk_meta_json))
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

    /// Delete a wake session
    pub fn delete_wake_session(&self, session_id: &str) -> Result<()> {
        Self::runtime().block_on(self.delete_wake_session_async(session_id))
    }

    async fn delete_wake_session_async(&self, session_id: &str) -> Result<()> {
        let mut response = with_db!(self, db, {
            db.query("DELETE type::thing('wake_session', $session_id)")
                .bind(("session_id", session_id.to_string()))
                .await
                .context("Failed to delete wake session")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB error deleting wake session: {:?}",
                errors
            ));
        }

        Ok(())
    }
}
