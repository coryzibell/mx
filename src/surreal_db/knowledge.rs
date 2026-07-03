use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

use crate::knowledge::KnowledgeEntry;

use super::connection::normalize_datetime;
use super::{RecordId, SurrealConnection, SurrealDatabase};

/// DTO for deserializing knowledge records from SurrealDB queries.
///
/// SurrealDB returns record links as `Thing` types, which don't deserialize
/// to serde_json::Value properly. This DTO expects queries to use:
///   - `meta::id(id) AS id` for the record ID
///   - `meta::id(category) AS category_id` for record links
///   - `<string>created_at AS created_at` for datetime conversion
///
/// This allows direct deserialization without manual JSON field extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SurrealKnowledgeRecord {
    /// Record ID (from `meta::id(id)`)
    pub id: String,

    /// Entry title
    pub title: String,

    /// Full body content
    #[serde(default)]
    pub body: Option<String>,

    /// Brief summary
    #[serde(default)]
    pub summary: Option<String>,

    /// Source file path (for markdown-sourced entries)
    #[serde(default)]
    pub file_path: Option<String>,

    /// Content hash for change detection
    #[serde(default)]
    pub content_hash: Option<String>,

    /// Whether this is ephemeral/session-scoped
    #[serde(default)]
    pub ephemeral: bool,

    /// Owner ID for private entries
    #[serde(default)]
    pub owner: Option<String>,

    /// Visibility: "public" or "private"
    #[serde(default = "default_visibility")]
    pub visibility: String,

    // === Record links (converted to strings via meta::id()) ===
    /// Category ID (from `meta::id(category)`)
    pub category_id: String,

    /// Source type ID (from `meta::id(source_type)`)
    #[serde(default)]
    pub source_type_id: Option<String>,

    /// Entry type ID (from `meta::id(entry_type)`)
    #[serde(default)]
    pub entry_type_id: Option<String>,

    /// Content type ID (from `meta::id(content_type)`)
    #[serde(default)]
    pub content_type_id: Option<String>,

    /// Source project ID
    #[serde(default)]
    pub source_project_id: Option<String>,

    /// Source agent ID
    #[serde(default)]
    pub source_agent_id: Option<String>,

    /// Session ID
    #[serde(default)]
    pub session_id: Option<String>,

    // === Timestamps (converted to strings via <string>cast) ===
    /// Created timestamp (from `<string>created_at`)
    #[serde(default)]
    pub created_at: Option<String>,

    /// Updated timestamp (from `<string>updated_at`)
    #[serde(default)]
    pub updated_at: Option<String>,

    // === Resonance fields (for wake-up cascade) ===
    /// Resonance level (1-10, with overflow for transcendent)
    #[serde(default)]
    pub resonance: i32,

    /// Resonance type: foundational, transformative, relational, operational, ephemeral
    #[serde(default)]
    pub resonance_type: Option<String>,

    /// Last activated timestamp
    #[serde(default)]
    pub last_activated: Option<String>,

    /// Number of times activated
    #[serde(default)]
    pub activation_count: i32,

    /// Decay rate (0.0-1.0)
    #[serde(default)]
    pub decay_rate: f64,

    /// Anchor IDs (related blooms this connects to)
    #[serde(default)]
    pub anchors: Vec<String>,

    // Issue #72: Multiple wake phrases
    #[serde(default)]
    pub wake_phrases: Vec<String>,

    // Issue #246: Trigger keywords for reactive memory injection
    #[serde(default)]
    pub triggers: Vec<String>,

    // Issue #73: Custom wake order
    #[serde(default)]
    pub wake_order: Option<i32>,

    /// DEPRECATED: Wake phrase for memory ritual verification (kept for backward compat)
    #[serde(default)]
    pub wake_phrase: Option<String>,

    // === Vector embeddings (PR #89) ===
    /// 768-dimensional embedding vector (BGE-Base-EN-v1.5)
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,

    /// Model ID that generated the embedding
    #[serde(default)]
    pub embedding_model: Option<String>,

    /// Timestamp when embedded
    #[serde(default)]
    pub embedded_at: Option<String>,

    // === Embedding chunks (Issue #346) ===
    /// Number of embedding chunks (0 = unchunked)
    #[serde(default)]
    pub chunk_count: i32,

    // === Stele encoding (Issue #122) ===
    /// Content format: markdown (default), json, stele:markdown, stele:ascii, stele:light, stele:full
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_visibility() -> String {
    "public".to_string()
}

fn default_format() -> String {
    "markdown".to_string()
}

impl SurrealKnowledgeRecord {
    /// Convert to domain KnowledgeEntry, fetching tags and applicability
    pub fn into_knowledge_entry(
        self,
        tags: Vec<String>,
        applicability: Vec<String>,
    ) -> KnowledgeEntry {
        KnowledgeEntry {
            id: format!("kn-{}", self.id),
            category_id: self.category_id,
            title: self.title,
            body: self.body,
            summary: self.summary,
            file_path: self.file_path,
            content_hash: self.content_hash,
            ephemeral: self.ephemeral,
            owner: self.owner,
            visibility: self.visibility,
            source_type_id: self.source_type_id,
            entry_type_id: self.entry_type_id,
            content_type_id: self.content_type_id,
            source_project_id: self.source_project_id,
            source_agent_id: self.source_agent_id,
            session_id: self.session_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tags,
            applicability,
            resonance: self.resonance,
            resonance_type: self.resonance_type,
            last_activated: self.last_activated,
            activation_count: self.activation_count,
            decay_rate: self.decay_rate,
            anchors: self.anchors,
            wake_phrases: self.wake_phrases,
            triggers: self.triggers,
            wake_order: self.wake_order,
            wake_phrase: self.wake_phrase,
            embedding: self.embedding,
            embedding_model: self.embedding_model,
            embedded_at: self.embedded_at,
            chunk_count: self.chunk_count,
            format: self.format,
            effective_resonance: None,
        }
    }
}

impl SurrealDatabase {
    /// Build standard knowledge entry SELECT fields
    pub(super) fn knowledge_select_fields() -> &'static str {
        "meta::id(id) AS id, title, body, summary, file_path, content_hash, ephemeral,
        owner, visibility,
        meta::id(category) AS category_id,
        meta::id(source_type) AS source_type_id,
        meta::id(entry_type) AS entry_type_id,
        meta::id(content_type) AS content_type_id,
        IF source_project THEN meta::id(source_project) ELSE null END AS source_project_id,
        IF source_agent THEN meta::id(source_agent) ELSE null END AS source_agent_id,
        IF session THEN meta::id(session) ELSE null END AS session_id,
        <string>created_at AS created_at, <string>updated_at AS updated_at,
        IF resonance THEN resonance ELSE 0 END AS resonance,
        IF resonance_type THEN <string>resonance_type ELSE null END AS resonance_type,
        IF last_activated THEN <string>last_activated ELSE null END AS last_activated,
        IF activation_count THEN activation_count ELSE 0 END AS activation_count,
        IF decay_rate THEN decay_rate ELSE 0.0 END AS decay_rate,
        IF anchors THEN anchors ELSE [] END AS anchors,
        IF wake_phrases THEN wake_phrases ELSE [] END AS wake_phrases,
        IF triggers THEN triggers ELSE [] END AS triggers,
        IF wake_order THEN wake_order ELSE null END AS wake_order,
        IF wake_phrase THEN wake_phrase ELSE null END AS wake_phrase,
        IF embedding THEN embedding ELSE null END AS embedding,
        IF embedding_model THEN embedding_model ELSE null END AS embedding_model,
        IF embedded_at THEN <string>embedded_at ELSE null END AS embedded_at,
        IF chunk_count THEN chunk_count ELSE 0 END AS chunk_count,
        IF format THEN format ELSE 'markdown' END AS format"
    }

    /// Build visibility filter for privacy-aware queries
    pub(super) fn build_visibility_filter(
        ctx: &crate::store::AgentContext,
    ) -> (String, Option<String>) {
        if ctx.include_private {
            if let Some(ref agent) = ctx.agent_id {
                (
                    "AND ((visibility = 'public') OR (visibility = 'private' AND owner = $current_agent))".to_string(),
                    Some(agent.clone())
                )
            } else {
                ("AND (visibility = 'public')".to_string(), None)
            }
        } else {
            ("AND (visibility = 'public')".to_string(), None)
        }
    }

    /// Returns the SurrealQL expression for computing effective_resonance with tiered decay.
    /// Single source of truth for the decay formula.
    ///
    /// Tiered decay rates:
    ///   resonance <= 3  -> 10%/week (base 0.90)
    ///   resonance 4-5   -> 5%/week  (base 0.95)
    ///   resonance 6+    -> 2.5%/week (base 0.975)
    /// foundational/transformative entries are exempt from decay (effective_resonance = resonance).
    ///
    /// All other resonance types -- including `session`, `ephemeral`, `relational`,
    /// and `operational` -- are subject to decay. `session` entries intentionally decay
    /// like ephemeral: they represent per-session context that should lose salience over
    /// time rather than persist at full resonance indefinitely.
    pub(super) fn effective_resonance_expr() -> &'static str {
        "IF resonance_type IN ['foundational', 'transformative'] THEN resonance \
         ELSE resonance * math::pow(\
             IF resonance <= 3 THEN 0.90 \
             ELSE IF resonance <= 5 THEN 0.95 \
             ELSE 0.975 \
             END, \
             duration::days(time::now() - (last_activated ?? created_at)) / 7.0\
         ) \
         END"
    }

    /// Compute effective resonance in Rust, matching the SQL in `effective_resonance_expr()`.
    ///
    /// Tiered decay rates (per week):
    ///   resonance <= 3  -> 10%/week (base 0.90)
    ///   resonance 4-5   -> 5%/week  (base 0.95)
    ///   resonance 6+    -> 2.5%/week (base 0.975)
    /// foundational/transformative entries are exempt from decay (return raw resonance).
    fn compute_effective_resonance(entry: &KnowledgeEntry) -> f64 {
        let resonance = entry.resonance as f64;

        // Foundational and transformative entries are exempt from decay
        if let Some(ref rtype) = entry.resonance_type
            && (rtype == "foundational" || rtype == "transformative")
        {
            return resonance;
        }

        // Determine the reference timestamp: last_activated, falling back to created_at
        let reference_ts = entry
            .last_activated
            .as_deref()
            .or(entry.created_at.as_deref());

        let weeks_elapsed = match reference_ts {
            Some(ts) => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    let elapsed = chrono::Utc::now() - dt.to_utc();
                    elapsed.num_seconds() as f64 / (7.0 * 86400.0)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };

        // Tiered decay base matching the SQL expression
        let decay_base: f64 = if entry.resonance <= 3 {
            0.90
        } else if entry.resonance <= 5 {
            0.95
        } else {
            0.975
        };

        resonance * decay_base.powf(weeks_elapsed)
    }

    /// Build resonance filter clauses using computed effective_resonance.
    /// Tiered decay rates:
    ///   resonance <= 3  -> 10%/week (base 0.90)
    ///   resonance 4-5   -> 5%/week  (base 0.95)
    ///   resonance 6+    -> 2.5%/week (base 0.975)
    /// foundational/transformative entries are exempt from decay.
    pub(super) fn build_resonance_filter(filter: &crate::store::KnowledgeFilter) -> String {
        // SurrealDB doesn't support LET in WHERE, so we expand the expression directly.
        let effective_resonance_expr = Self::effective_resonance_expr();

        let mut clauses = Vec::new();

        if let Some(min) = filter.min_resonance {
            clauses.push(format!("({}) >= {}", effective_resonance_expr, min));
        }

        if let Some(max) = filter.max_resonance {
            clauses.push(format!("({}) <= {}", effective_resonance_expr, max));
        }

        if clauses.is_empty() {
            String::new()
        } else {
            format!("AND ({})", clauses.join(" AND "))
        }
    }

    /// Validate category name to prevent SQL injection
    /// Only allows alphanumeric characters, underscores, and hyphens
    fn is_valid_category_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    /// Build category filter clauses
    /// Category names are validated to prevent SQL injection
    pub(super) fn build_category_filter(filter: &crate::store::KnowledgeFilter) -> String {
        match &filter.categories {
            Some(cats) if !cats.is_empty() => {
                // Filter out invalid category names to prevent injection
                let valid_cats: Vec<&String> = cats
                    .iter()
                    .filter(|c| Self::is_valid_category_name(c))
                    .collect();

                if valid_cats.is_empty() {
                    return String::new();
                }

                if valid_cats.len() == 1 {
                    format!(
                        "AND category = type::thing('category', '{}')",
                        valid_cats[0]
                    )
                } else {
                    // Multiple categories: use IN clause
                    let quoted: Vec<String> = valid_cats
                        .iter()
                        .map(|c| format!("type::thing('category', '{}')", c))
                        .collect();
                    format!("AND category IN [{}]", quoted.join(", "))
                }
            }
            _ => String::new(),
        }
    }

    /// Build a WHERE-clause fragment (candidate-set level, applied BEFORE any
    /// `LIMIT`) that excludes any `knowledge` row carrying a tag whose name
    /// prefix-matches one of `exclude_prefixes`.
    ///
    /// Returns `("", vec![])` (no exclusion) when `exclude_prefixes` is empty.
    /// Otherwise returns the SQL fragment plus the `(param_name, value)` pairs
    /// the caller MUST bind — one bound parameter per prefix (`exclude_prefix_0`,
    /// `exclude_prefix_1`, ...). Only parameter NAMES are interpolated into the
    /// SQL text; prefix VALUES are always bound, never string-interpolated
    /// (review fix — this is the archive-tier exclusion filter and prefixes
    /// are caller-controlled input).
    ///
    /// Traverses the same `->tagged_with->tag` graph edge already used to
    /// project the `tags` field (see the `SELECT VALUE out.name FROM
    /// tagged_with` subquery at :1317), so this reuses a verified traversal
    /// direction rather than inventing a new one.
    pub(super) fn build_exclude_tags_filter(
        exclude_prefixes: &[String],
    ) -> (String, Vec<(String, String)>) {
        if exclude_prefixes.is_empty() {
            return (String::new(), Vec::new());
        }

        let mut bindings = Vec::with_capacity(exclude_prefixes.len());
        let mut conditions = Vec::with_capacity(exclude_prefixes.len());
        for (i, prefix) in exclude_prefixes.iter().enumerate() {
            let param = format!("exclude_prefix_{i}");
            conditions.push(format!("string::starts_with(name, ${param})"));
            bindings.push((param, prefix.clone()));
        }

        let clause = format!(
            "AND array::len(->tagged_with->tag[WHERE {}]) = 0",
            conditions.join(" OR ")
        );
        (clause, bindings)
    }

    // =========================================================================
    // KNOWLEDGE CRUD OPERATIONS
    // =========================================================================

    /// Upsert a knowledge entry with tags and applicability edges (returns RecordId)
    pub fn upsert_knowledge_internal(&self, entry: &KnowledgeEntry) -> Result<RecordId> {
        Self::runtime().block_on(self.upsert_knowledge_async(entry))
    }

    async fn upsert_knowledge_async(&self, entry: &KnowledgeEntry) -> Result<RecordId> {
        // Extract ID from "kn-xxxxx" format
        let id_part = entry.id.strip_prefix("kn-").unwrap_or(&entry.id);
        let record_id = RecordId::new("knowledge", id_part);

        // Build base query with required fields
        let mut query = "UPSERT type::thing('knowledge', $id) SET
            title = $title,
            body = $body,
            summary = $summary,
            file_path = $file_path,
            content_hash = $content_hash,
            ephemeral = $ephemeral,
            owner = $owner,
            visibility = $visibility,
            category = type::thing('category', $category_id),
            source_type = type::thing('source_type', $source_type_id),
            entry_type = type::thing('entry_type', $entry_type_id),
            content_type = type::thing('content_type', $content_type_id),
            resonance = $resonance,
            resonance_type = $resonance_type,
            activation_count = $activation_count,
            decay_rate = $decay_rate,
            anchors = $anchors,
            wake_phrases = $wake_phrases,
            triggers = $triggers,
            wake_order = $wake_order,
            wake_phrase = $wake_phrase,
            embedding = $embedding,
            embedding_model = $embedding_model,
            chunk_count = $chunk_count,
            format = $format"
            .to_string();

        // Add optional fields
        if entry.source_project_id.is_some() {
            query.push_str(", source_project = type::thing('project', $source_project_id)");
        }
        if entry.source_agent_id.is_some() {
            query.push_str(", source_agent = type::thing('agent', $source_agent_id)");
        }
        if entry.session_id.is_some() {
            query.push_str(", session = type::thing('session', $session_id)");
        }
        if entry.created_at.is_some() {
            query.push_str(", created_at = <datetime>$created_at");
        }
        if entry.updated_at.is_some() {
            query.push_str(", updated_at = <datetime>$updated_at");
        }
        if entry.last_activated.is_some() {
            query.push_str(", last_activated = <datetime>$last_activated");
        }
        if entry.embedded_at.is_some() {
            query.push_str(", embedded_at = <datetime>$embedded_at");
        }

        // Bind all parameters and execute query
        let mut response = with_db!(self, db, {
            let mut q = db
                .query(&query)
                .bind(("id", id_part.to_string()))
                .bind(("title", entry.title.clone()))
                .bind(("body", entry.body.clone()))
                .bind(("summary", entry.summary.clone()))
                .bind(("file_path", entry.file_path.clone()))
                .bind((
                    "content_hash",
                    entry.content_hash.clone().unwrap_or_default(),
                ))
                .bind(("ephemeral", entry.ephemeral))
                .bind(("owner", entry.owner.clone()))
                .bind(("visibility", entry.visibility.clone()))
                .bind(("category_id", entry.category_id.clone()))
                .bind((
                    "source_type_id",
                    entry
                        .source_type_id
                        .clone()
                        .unwrap_or_else(|| "manual".to_string()),
                ))
                .bind((
                    "entry_type_id",
                    entry
                        .entry_type_id
                        .clone()
                        .unwrap_or_else(|| "primary".to_string()),
                ))
                .bind((
                    "content_type_id",
                    entry
                        .content_type_id
                        .clone()
                        .unwrap_or_else(|| "text".to_string()),
                ))
                .bind(("resonance", entry.resonance))
                .bind(("resonance_type", entry.resonance_type.clone()))
                .bind(("activation_count", entry.activation_count))
                .bind(("decay_rate", entry.decay_rate))
                .bind(("anchors", entry.anchors.clone()))
                .bind(("wake_phrases", entry.wake_phrases.clone()))
                // Issue #246: normalize triggers on write (lowercase + collapse
                // whitespace, drop empties, dedupe) so author-time values line up
                // with match-time lookups in PR3. Bound param, never interpolated.
                .bind((
                    "triggers",
                    crate::knowledge::normalize_triggers(entry.triggers.iter()),
                ))
                .bind(("wake_order", entry.wake_order))
                .bind(("wake_phrase", entry.wake_phrase.clone()))
                .bind(("embedding", entry.embedding.clone()))
                .bind(("embedding_model", entry.embedding_model.clone()))
                .bind(("chunk_count", entry.chunk_count))
                .bind(("format", entry.format.clone()));

            // Bind optional parameters
            if let Some(ref proj) = entry.source_project_id {
                q = q.bind(("source_project_id", proj.clone()));
            }
            if let Some(ref agent) = entry.source_agent_id {
                q = q.bind(("source_agent_id", agent.clone()));
            }
            if let Some(ref sess) = entry.session_id {
                q = q.bind(("session_id", sess.clone()));
            }
            if let Some(ref created) = entry.created_at {
                q = q.bind(("created_at", normalize_datetime(created)));
            }
            if let Some(ref updated) = entry.updated_at {
                q = q.bind(("updated_at", normalize_datetime(updated)));
            }
            if let Some(ref activated) = entry.last_activated {
                q = q.bind(("last_activated", normalize_datetime(activated)));
            }
            if let Some(ref embedded) = entry.embedded_at {
                q = q.bind(("embedded_at", normalize_datetime(embedded)));
            }

            q.await.context("Failed to upsert knowledge record")
        })?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        // Manage tags - delete old, create new
        let mut tag_delete_response = with_db!(self, db, {
            db.query("DELETE tagged_with WHERE in = $knowledge")
                .bind(("knowledge", record_id.0.clone()))
                .await
                .context("Failed to clear existing tags")
        })?;

        let tag_delete_errors = tag_delete_response.take_errors();
        if !tag_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                tag_delete_errors
            ));
        }

        for tag_name in &entry.tags {
            // Ensure tag exists - use query UPSERT to handle schema defaults
            let mut tag_response = with_db!(self, db, {
                db.query("UPSERT type::thing('tag', $tag_id) SET name = $tag_name")
                    .bind(("tag_id", tag_name.clone()))
                    .bind(("tag_name", tag_name.clone()))
                    .await
                    .context("Failed to create tag")
            })?;

            let tag_errors = tag_response.take_errors();
            if !tag_errors.is_empty() {
                return Err(anyhow::anyhow!("Failed to create tag: {:?}", tag_errors));
            }

            let tag_id = RecordId::new("tag", tag_name);

            // Create edge
            let mut tag_edge_response = with_db!(self, db, {
                db.query("RELATE $knowledge->tagged_with->$tag")
                    .bind(("knowledge", record_id.0.clone()))
                    .bind(("tag", tag_id.0.clone()))
                    .await
                    .context("Failed to create tag edge")
            })?;

            let tag_edge_errors = tag_edge_response.take_errors();
            if !tag_edge_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "SurrealDB returned errors: {:?}",
                    tag_edge_errors
                ));
            }
        }

        // Manage applicability - delete old, create new
        let mut app_delete_response = with_db!(self, db, {
            db.query("DELETE applies_to WHERE in = $knowledge")
                .bind(("knowledge", record_id.0.clone()))
                .await
                .context("Failed to clear existing applicability")
        })?;

        let app_delete_errors = app_delete_response.take_errors();
        if !app_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                app_delete_errors
            ));
        }

        for app_type in &entry.applicability {
            // Ensure applicability_type exists - use query UPSERT to handle schema defaults
            let mut app_type_response = with_db!(self, db, {
                db.query("UPSERT type::thing('applicability_type', $app_type_id) SET description = $app_type_desc")
                    .bind(("app_type_id", app_type.clone()))
                    .bind(("app_type_desc", format!("Applicability: {}", app_type)))
                    .await
                    .context("Failed to create applicability_type")
            })?;

            let app_type_errors = app_type_response.take_errors();
            if !app_type_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "Failed to create applicability_type: {:?}",
                    app_type_errors
                ));
            }

            let app_id = RecordId::new("applicability_type", app_type);

            // Create edge
            let mut app_edge_response = with_db!(self, db, {
                db.query("RELATE $knowledge->applies_to->$app_type")
                    .bind(("knowledge", record_id.0.clone()))
                    .bind(("app_type", app_id.0.clone()))
                    .await
                    .context("Failed to create applicability edge")
            })?;

            let app_edge_errors = app_edge_response.take_errors();
            if !app_edge_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "SurrealDB returned errors: {:?}",
                    app_edge_errors
                ));
            }
        }

        Ok(record_id)
    }

    /// Get a knowledge entry by ID
    pub fn get_knowledge(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Option<KnowledgeEntry>> {
        Self::runtime().block_on(self.get_knowledge_async(id, ctx))
    }

    async fn get_knowledge_async(
        &self,
        id: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Option<KnowledgeEntry>> {
        let id_part = id.strip_prefix("kn-").unwrap_or(id);

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE meta::id(id) = $id {}",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("id", id_part.to_string()));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query knowledge record")
        })?;

        // Direct deserialization to DTO - no manual JSON parsing!
        let records: Vec<SurrealKnowledgeRecord> = response.take(0)?;

        if records.is_empty() {
            return Ok(None);
        }

        let record = records.into_iter().next().unwrap();

        // Fetch tags and applicability separately
        let tags = self
            .get_tags_for_entry_async(&format!("kn-{}", record.id))
            .await?;
        let applicability = self
            .get_applicability_for_entry_async(&format!("kn-{}", record.id))
            .await?;

        Ok(Some(record.into_knowledge_entry(tags, applicability)))
    }

    /// Delete a knowledge entry (edges cascade automatically).
    /// Respects visibility: agents can only delete entries they can see.
    /// Returns Ok(false) for entries that don't exist OR that the agent can't see
    /// (to avoid leaking existence of private entries).
    pub fn delete_knowledge(&self, id: &str, ctx: &crate::store::AgentContext) -> Result<bool> {
        Self::runtime().block_on(self.delete_knowledge_async(id, ctx))
    }

    async fn delete_knowledge_async(
        &self,
        id: &str,
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
                .context("Failed to check knowledge record existence")
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

        // Delete with the same visibility filter to prevent TOCTOU race conditions.
        // Even though we checked above, re-applying the filter on the DELETE ensures
        // no bypass is possible between check and delete.
        let delete_sql = format!(
            "DELETE FROM knowledge WHERE meta::id(id) = $id {}",
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&delete_sql).bind(("id", id_part.to_string()));
            if let Some(ref agent) = current_agent {
                query = query.bind(("current_agent", agent.clone()));
            }
            query.await.context("Failed to delete knowledge record")
        })?;

        // Check for errors
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Delete failed: {:?}", errors));
        }

        // Best-effort cleanup of embedding chunks (Issue #346)
        let full_entry_id = format!("kn-{}", id_part);
        self.delete_embedding_chunks_async(&full_entry_id)
            .await
            .ok();

        Ok(true)
    }

    /// Search knowledge using BM25 full-text indexes
    pub fn search_knowledge(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.search_knowledge_async(query, ctx, filter))
    }

    async fn search_knowledge_async(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        let query_owned = query.to_string();

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);
        let category_clause = Self::build_category_filter(filter);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE (title @@ $query OR body @@ $query OR summary @@ $query) {} {} {}",
            Self::knowledge_select_fields(),
            visibility_clause,
            resonance_clause,
            category_clause
        );

        let mut response = with_db!(self, db, {
            let mut query_builder = db.query(&sql).bind(("query", query_owned));
            if let Some(agent) = current_agent {
                query_builder = query_builder.bind(("current_agent", agent));
            }
            query_builder
                .await
                .context("Failed to execute search query")
        })?;

        let results: Vec<serde_json::Value> =
            response.take(0).context("Failed to parse search results")?;

        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Semantic search using vector similarity (brute force cosine).
    ///
    /// Public entry point used by `mx memory search`. Returns entries ordered by
    /// descending similarity, with scores discarded. Behavior is unchanged: this
    /// now delegates to [`semantic_search_knowledge_scored`] and drops the score,
    /// so callers see byte-identical results.
    pub fn semantic_search_knowledge(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        Ok(self
            .semantic_search_knowledge_scored(query_embedding, ctx, filter, limit)?
            .into_iter()
            .map(|(entry, _score)| entry)
            .collect())
    }

    /// Scored variant of [`semantic_search_knowledge`]: returns `(entry, score)`
    /// pairs ordered by descending cosine similarity. This is the shared core
    /// that the public, score-discarding method delegates to — exposing the
    /// `score` that the SQL already computes (Issue #362) without changing the
    /// public search contract.
    ///
    /// Like the public method, this matches at both the entry level (mean-pooled
    /// `embedding`) and the chunk level (`embedding_chunk`), deduplicating to one
    /// row per entry by taking the max score (Issue #346).
    pub fn semantic_search_knowledge_scored(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<(KnowledgeEntry, f32)>> {
        Self::runtime().block_on(self.semantic_search_knowledge_async(
            query_embedding,
            ctx,
            filter,
            limit,
        ))
    }

    async fn semantic_search_knowledge_async(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<(KnowledgeEntry, f32)>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);
        let category_clause = Self::build_category_filter(filter);
        // Review fix: exclude archived (or any prefix-matched) tags at the
        // candidate-set level, BEFORE the unchunked query's own `LIMIT`, so a
        // ranked-but-excluded neighbor never displaces a non-excluded one out
        // of the top `limit` rows the DB returns.
        let (exclude_clause, exclude_bindings) =
            Self::build_exclude_tags_filter(&filter.exclude_tag_prefixes);

        // Dimension guard ($dim, bound below): SurrealDB's
        // vector::similarity::cosine ABORTS THE ENTIRE SCAN when it hits any
        // row whose embedding dimension differs from the query vector. One
        // off-dim row (e.g. a leaked dim-4 test fixture) therefore breaks ALL
        // search. Skip such rows defensively by requiring the stored embedding
        // length to match the query length BEFORE the cosine is scored. Real
        // rows are 768 and the query is 768, so normal search is unaffected;
        // only mismatched rows are filtered out instead of aborting the scan.
        let query_dim = query_embedding.len();

        // Phase 1a: Search unchunked entries (chunk_count <= 0 or absent).
        // Exclusion is pushed into this query's own WHERE (bound params, see
        // build_exclude_tags_filter), so an excluded row never occupies a
        // slot in the DB's own LIMIT.
        let unchunked_sql = format!(
            "SELECT {}, vector::similarity::cosine(embedding, $query_vec) AS score
            FROM knowledge
            WHERE embedding IS NOT NONE AND array::len(embedding) = $dim AND (chunk_count IS NONE OR chunk_count <= 0) {} {} {} {}
            ORDER BY score DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause,
            resonance_clause,
            category_clause,
            exclude_clause
        );

        let mut unchunked_response = with_db!(self, db, {
            let mut query_builder = db
                .query(&unchunked_sql)
                .bind(("query_vec", query_embedding.to_vec()))
                .bind(("dim", query_dim))
                .bind(("limit", limit));
            if let Some(agent) = current_agent.clone() {
                query_builder = query_builder.bind(("current_agent", agent));
            }
            for (param, value) in exclude_bindings.clone() {
                query_builder = query_builder.bind((param, value));
            }
            query_builder
                .await
                .context("Failed to execute semantic search query")
        })?;

        let unchunked_results: Vec<serde_json::Value> = unchunked_response
            .take(0)
            .context("Failed to parse unchunked search results")?;

        // Phase 2a: Collect unchunked entries with their scores.
        let mut scored_entries: std::collections::HashMap<String, (f32, Option<KnowledgeEntry>)> =
            std::collections::HashMap::new();
        for obj in unchunked_results {
            let entry = self.value_to_knowledge_entry(obj.clone()).await?;
            let score = obj["score"].as_f64().unwrap_or(0.0) as f32;
            scored_entries.insert(entry.id.clone(), (score, Some(entry)));
        }

        // Phase 1b/2b: Search chunks (no visibility filter — applied after
        // hydration below) and merge into `scored_entries`.
        //
        // `embedding_chunk` has no direct tag link (it's keyed on a plain
        // `entry_id` string, not a graph edge to `knowledge`), so exclusion
        // can't be pushed into this query's own WHERE the way it is for the
        // unchunked phase above. A single fixed-size over-fetch (`limit * N`)
        // is a fixed-ratio post-filter: it silently returns fewer than
        // `limit` non-excluded results once the excluded (e.g. archived)
        // share of chunk candidates exceeds that ratio — and the archive
        // tier is a default exclusion designed to GROW without bound (per
        // work order), so a static ratio isn't safe long-term.
        //
        // Instead, over-fetch iteratively: start at `limit * 3`, hydrate and
        // filter each newly-seen candidate, and if the merged non-excluded
        // count is still short of `limit` AND the table had more chunk rows
        // to give (`returned_count == chunk_fetch`, i.e. we weren't already
        // at the end of the table), double the fetch window and try again —
        // up to a hard cap so a pathological query can't force an unbounded
        // scan.
        let mut chunk_fetch = limit.saturating_mul(3).max(1);
        let max_chunk_fetch = limit.saturating_mul(50).max(1000);
        // Cache of every embedding_chunk entry_id already resolved (kept,
        // with its score, or dropped by a filter) so growing the over-fetch
        // window never re-hydrates or re-filters the same entry twice.
        let mut resolved_chunk_entries: std::collections::HashMap<
            String,
            Option<(f32, KnowledgeEntry)>,
        > = std::collections::HashMap::new();

        loop {
            // Same dimension guard as the unchunked phase so an off-dim
            // chunk embedding can't abort the scan.
            let chunk_sql =
                "SELECT entry_id, vector::similarity::cosine(embedding, $query_vec) AS score
                FROM embedding_chunk
                WHERE array::len(embedding) = $dim
                ORDER BY score DESC
                LIMIT $chunk_limit";

            let mut chunk_response = with_db!(self, db, {
                db.query(chunk_sql)
                    .bind(("query_vec", query_embedding.to_vec()))
                    .bind(("dim", query_dim))
                    .bind(("chunk_limit", chunk_fetch))
                    .await
                    .context("Failed to execute chunk semantic search query")
            })?;

            let chunk_results: Vec<serde_json::Value> = chunk_response
                .take(0)
                .context("Failed to parse chunk search results")?;
            let returned_count = chunk_results.len();

            // Deduplicate chunks: keep max score per entry_id, over this
            // fetch window.
            let mut chunk_scores: std::collections::HashMap<String, f32> =
                std::collections::HashMap::new();
            for obj in &chunk_results {
                let entry_id = obj["entry_id"].as_str().unwrap_or_default().to_string();
                let score = obj["score"].as_f64().unwrap_or(0.0) as f32;
                let current = chunk_scores.entry(entry_id).or_insert(0.0f32);
                if score > *current {
                    *current = score;
                }
            }

            // For each unique chunk entry_id not yet accounted for, fetch
            // the full entry (with visibility/category/resonance/exclude
            // filtering), exactly once per entry_id across all iterations.
            for (entry_id, score) in &chunk_scores {
                if scored_entries.contains_key(entry_id) {
                    // Entry already in results from unchunked path — take max score
                    if let Some((existing_score, _)) = scored_entries.get_mut(entry_id)
                        && *score > *existing_score
                    {
                        *existing_score = *score;
                    }
                    continue;
                }
                if resolved_chunk_entries.contains_key(entry_id) {
                    // Already hydrated and filtered in a prior (smaller)
                    // fetch window — the verdict doesn't change, skip.
                    continue;
                }

                let resolved = if let Some(entry) = self.get_knowledge_async(entry_id, ctx).await? {
                    // Apply decay-adjusted resonance filter (matching the SQL in effective_resonance_expr)
                    let effective = Self::compute_effective_resonance(&entry);
                    let passes_resonance = filter
                        .min_resonance
                        .is_none_or(|min| effective >= min as f64)
                        && filter
                            .max_resonance
                            .is_none_or(|max| effective <= max as f64);
                    // Apply category filter
                    let passes_category = filter
                        .categories
                        .as_ref()
                        .is_none_or(|cats| cats.is_empty() || cats.contains(&entry.category_id));
                    // Review fix, embedding_chunk phase: filter here, before
                    // this candidate enters `scored_entries`, using the
                    // now-hydrated entry's tags. This keeps exclusion at
                    // candidate-set-construction time, before the final
                    // sort+truncate(limit) below, matching the
                    // resonance/category filters above.
                    let passes_exclude = crate::helpers::keep_after_exclude(
                        &entry.tags,
                        &filter.exclude_tag_prefixes,
                    );
                    if passes_resonance && passes_category && passes_exclude {
                        Some((*score, entry))
                    } else {
                        None
                    }
                } else {
                    // Entry not found or not visible.
                    None
                };
                resolved_chunk_entries.insert(entry_id.clone(), resolved);
            }

            // Merge every kept, resolved chunk entry into `scored_entries`.
            for (entry_id, resolved) in &resolved_chunk_entries {
                if let Some((score, entry)) = resolved {
                    scored_entries
                        .entry(entry_id.clone())
                        .and_modify(|(existing_score, _)| {
                            if *score > *existing_score {
                                *existing_score = *score;
                            }
                        })
                        .or_insert_with(|| (*score, Some(entry.clone())));
                }
            }

            let have_enough = scored_entries.len() >= limit;
            let exhausted = returned_count < chunk_fetch; // fewer rows than asked => table has no more
            let capped = chunk_fetch >= max_chunk_fetch;
            if capped && !have_enough && !exhausted {
                // The scan budget ran out before the candidate set filled to
                // `limit` — distinct from `exhausted` (table genuinely has no
                // more rows). This is the excluded-tag-dominance case: enough
                // chunk candidates outrank the eligible ones that `limit*50`
                // over-fetch iterations never surface `limit` non-excluded
                // entries. Callers relying on a full page need to know the
                // result is short.
                eprintln!(
                    "Warning: semantic search hit chunk scan cap ({max_chunk_fetch} candidates) \
                     with only {} of {limit} requested results — likely excluded-tag dominance \
                     in the candidate set",
                    scored_entries.len()
                );
            }
            if have_enough || exhausted || capped {
                break;
            }
            chunk_fetch = chunk_fetch.saturating_mul(2).min(max_chunk_fetch);
        }

        // Sort by score DESC and take limit
        let mut sorted: Vec<(f32, KnowledgeEntry)> = scored_entries
            .into_values()
            .filter_map(|(score, entry)| entry.map(|e| (score, e)))
            .collect();
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(limit);

        Ok(sorted
            .into_iter()
            .map(|(score, entry)| (entry, score))
            .collect())
    }

    /// Entry-level scored vector search used by `auto_anchor` (Issue #362).
    ///
    /// Unlike [`semantic_search_knowledge_scored`], this scores ONLY against the
    /// entry-level (mean-pooled) `embedding` column on the `knowledge` table and
    /// does NOT consult `embedding_chunk`. It also imposes no `chunk_count`
    /// restriction, so long (chunked) entries are scored on their mean vector
    /// just like short ones.
    ///
    /// This exactly mirrors what the old `auto_anchor` did when it called
    /// `list_all` and ran `cosine_similarity(entry.embedding, candidate.embedding)`
    /// over every row — only now the cosine work runs DB-side and is bounded to
    /// the top `limit` rows by score, instead of hydrating the whole graph and
    /// looping in Rust. Anchoring therefore keeps strict entry-level-mean
    /// semantics (no chunk-level matching leaks into anchor selection).
    ///
    /// Returns `(entry, score)` pairs ordered by descending cosine similarity.
    pub fn semantic_search_entries_scored(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        limit: usize,
    ) -> Result<Vec<(KnowledgeEntry, f32)>> {
        Self::runtime().block_on(self.semantic_search_entries_scored_async(
            query_embedding,
            ctx,
            limit,
        ))
    }

    async fn semantic_search_entries_scored_async(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        limit: usize,
    ) -> Result<Vec<(KnowledgeEntry, f32)>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Dimension guard ($dim): see semantic_search_knowledge_async — a single
        // off-dim row otherwise aborts the whole cosine scan, breaking auto_anchor
        // (#362) entirely. Require the stored embedding length to match the query
        // length before scoring; real 768-dim rows pass, mismatches are skipped.
        let query_dim = query_embedding.len();

        // Single-phase: score every embedded knowledge row on its entry-level
        // vector, bounded to the top `limit` by score. Params are bound, never
        // interpolated.
        let sql = format!(
            "SELECT {}, vector::similarity::cosine(embedding, $query_vec) AS score
            FROM knowledge
            WHERE embedding IS NOT NONE AND array::len(embedding) = $dim {}
            ORDER BY score DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause,
        );

        let mut response = with_db!(self, db, {
            let mut query_builder = db
                .query(&sql)
                .bind(("query_vec", query_embedding.to_vec()))
                .bind(("dim", query_dim))
                .bind(("limit", limit));
            if let Some(agent) = current_agent.clone() {
                query_builder = query_builder.bind(("current_agent", agent));
            }
            query_builder
                .await
                .context("Failed to execute entry-level scored search query")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse entry-level scored search results")?;

        let mut scored = Vec::with_capacity(results.len());
        for obj in results {
            let score = obj["score"].as_f64().unwrap_or(0.0) as f32;
            let entry = self.value_to_knowledge_entry(obj).await?;
            scored.push((entry, score));
        }

        Ok(scored)
    }

    /// Helper: Convert SurrealDB query result to KnowledgeEntry
    pub(super) async fn value_to_knowledge_entry(
        &self,
        obj: serde_json::Value,
    ) -> Result<KnowledgeEntry> {
        // Extract ID from string (queries use meta::id(id) AS id)
        let id_str = obj["id"].as_str().unwrap_or_default();
        let id = format!("kn-{}", id_str);

        // Extract category ID from string field
        let category_id = obj["category_id"].as_str().unwrap_or_default().to_string();

        // Extract optional string fields for record links
        let source_project_id = obj
            .get("source_project_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let source_agent_id = obj
            .get("source_agent_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let session_id = obj
            .get("session_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let source_type_id = obj
            .get("source_type_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let entry_type_id = obj
            .get("entry_type_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let content_type_id = obj
            .get("content_type_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Fetch tags
        let knowledge_thing = Thing::from(("knowledge", id_str));
        let mut tags_response = with_db!(self, db, {
            db.query("SELECT VALUE out.name FROM tagged_with WHERE in = $knowledge")
                .bind(("knowledge", knowledge_thing.clone()))
                .await
                .context("Failed to query tags")
        })?;
        let tags: Vec<String> = tags_response.take(0).unwrap_or_default();

        // Fetch applicability
        let mut app_response = with_db!(self, db, {
            db.query("SELECT VALUE meta::id(out) FROM applies_to WHERE in = $knowledge")
                .bind(("knowledge", knowledge_thing))
                .await
                .context("Failed to query applicability")
        })?;
        let applicability_raw: Vec<Thing> = app_response.take(0).unwrap_or_default();
        let applicability: Vec<String> = applicability_raw
            .into_iter()
            .map(|t| t.id.to_string())
            .collect();

        Ok(KnowledgeEntry {
            id,
            category_id,
            title: serde_json::from_value(obj["title"].clone()).unwrap_or_default(),
            body: serde_json::from_value(obj["body"].clone()).ok(),
            summary: serde_json::from_value(obj["summary"].clone()).ok(),
            file_path: serde_json::from_value(obj["file_path"].clone()).ok(),
            content_hash: serde_json::from_value(obj["content_hash"].clone()).ok(),
            ephemeral: serde_json::from_value(obj["ephemeral"].clone()).unwrap_or(false),
            created_at: serde_json::from_value(obj["created_at"].clone()).ok(),
            updated_at: serde_json::from_value(obj["updated_at"].clone()).ok(),
            tags,
            applicability,
            source_project_id,
            source_agent_id,
            source_type_id,
            entry_type_id,
            content_type_id,
            session_id,
            owner: serde_json::from_value(obj["owner"].clone()).ok(),
            visibility: serde_json::from_value(obj["visibility"].clone())
                .unwrap_or_else(|_| "public".to_string()),
            resonance: serde_json::from_value(obj["resonance"].clone()).unwrap_or(0),
            resonance_type: serde_json::from_value(obj["resonance_type"].clone()).ok(),
            last_activated: serde_json::from_value(obj["last_activated"].clone()).ok(),
            activation_count: serde_json::from_value(obj["activation_count"].clone()).unwrap_or(0),
            decay_rate: serde_json::from_value(obj["decay_rate"].clone()).unwrap_or(0.0),
            anchors: serde_json::from_value(obj["anchors"].clone()).unwrap_or_default(),
            wake_phrases: serde_json::from_value(obj["wake_phrases"].clone()).unwrap_or_default(),
            triggers: serde_json::from_value(obj["triggers"].clone()).unwrap_or_default(),
            wake_order: serde_json::from_value(obj["wake_order"].clone()).ok(),
            wake_phrase: serde_json::from_value(obj["wake_phrase"].clone()).ok(),
            embedding: serde_json::from_value(obj["embedding"].clone()).ok(),
            embedding_model: serde_json::from_value(obj["embedding_model"].clone()).ok(),
            embedded_at: serde_json::from_value(obj["embedded_at"].clone()).ok(),
            chunk_count: serde_json::from_value(obj["chunk_count"].clone()).unwrap_or(0),
            format: serde_json::from_value(obj["format"].clone())
                .unwrap_or_else(|_| "markdown".to_string()),
            effective_resonance: obj.get("effective_resonance").and_then(|v| v.as_f64()),
        })
    }

    // =========================================================================
    // EMBEDDING CHUNK OPERATIONS (Issue #346)
    // =========================================================================

    /// Delete all embedding chunks for a knowledge entry (sync wrapper)
    pub fn delete_embedding_chunks(&self, entry_id: &str) -> Result<()> {
        Self::runtime().block_on(self.delete_embedding_chunks_async(entry_id))
    }

    async fn delete_embedding_chunks_async(&self, entry_id: &str) -> Result<()> {
        let mut response = with_db!(self, db, {
            db.query("DELETE FROM embedding_chunk WHERE entry_id = $entry_id")
                .bind(("entry_id", entry_id.to_string()))
                .await
                .context("Failed to delete embedding chunks")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to delete embedding chunks: {:?}",
                errors
            ));
        }

        Ok(())
    }

    /// Insert a single embedding chunk (sync wrapper)
    #[allow(clippy::too_many_arguments)]
    pub fn insert_embedding_chunk(
        &self,
        entry_id: &str,
        chunk_index: usize,
        chunk_text: &str,
        token_offset: usize,
        token_count: usize,
        embedding: &[f32],
        model_id: &str,
    ) -> Result<()> {
        Self::runtime().block_on(self.insert_embedding_chunk_async(
            entry_id,
            chunk_index,
            chunk_text,
            token_offset,
            token_count,
            embedding,
            model_id,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_embedding_chunk_async(
        &self,
        entry_id: &str,
        chunk_index: usize,
        chunk_text: &str,
        token_offset: usize,
        token_count: usize,
        embedding: &[f32],
        model_id: &str,
    ) -> Result<()> {
        let chunk_id = format!("{}_{}", entry_id, chunk_index);
        let sql = "UPSERT type::thing('embedding_chunk', $chunk_id) SET
            entry_id = $entry_id,
            chunk_index = $chunk_index,
            chunk_text = $chunk_text,
            token_offset = $token_offset,
            token_count = $token_count,
            embedding = $embedding,
            embedding_model = $embedding_model";

        let mut response = with_db!(self, db, {
            db.query(sql)
                .bind(("chunk_id", chunk_id))
                .bind(("entry_id", entry_id.to_string()))
                .bind(("chunk_index", chunk_index as i64))
                .bind(("chunk_text", chunk_text.to_string()))
                .bind(("token_offset", token_offset as i64))
                .bind(("token_count", token_count as i64))
                .bind(("embedding", embedding.to_vec()))
                .bind(("embedding_model", model_id.to_string()))
                .await
                .context("Failed to insert embedding chunk")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to insert embedding chunk: {:?}",
                errors
            ));
        }

        Ok(())
    }

    /// Search embedding chunks by vector similarity (sync wrapper)
    pub fn semantic_search_chunks(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        Self::runtime().block_on(self.semantic_search_chunks_async(query_embedding, limit))
    }

    async fn semantic_search_chunks_async(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        // Dimension guard ($dim): see semantic_search_knowledge_async — a single
        // off-dim chunk embedding otherwise aborts the entire cosine scan. Require
        // the stored embedding length to match the query length before scoring.
        let query_dim = query_embedding.len();

        let sql = "SELECT entry_id, vector::similarity::cosine(embedding, $query_vec) AS score
            FROM embedding_chunk
            WHERE array::len(embedding) = $dim
            ORDER BY score DESC
            LIMIT $limit";

        let mut response = with_db!(self, db, {
            db.query(sql)
                .bind(("query_vec", query_embedding.to_vec()))
                .bind(("dim", query_dim))
                .bind(("limit", limit))
                .await
                .context("Failed to search embedding chunks")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse chunk search results")?;

        let mut pairs = Vec::new();
        for obj in results {
            let entry_id = obj["entry_id"].as_str().unwrap_or_default().to_string();
            let score = obj["score"].as_f64().unwrap_or(0.0) as f32;
            pairs.push((entry_id, score));
        }

        Ok(pairs)
    }

    // ------------------------------------------------------------------
    // Test-only raw helpers (Issue #352 / W1 regression coverage).
    //
    // These exist so the chunk_count backfill test can drive a raw
    // statement and read the stored value back WITHOUT going through the
    // read-coalescing projection (which would mask a NONE as 0). They are
    // compiled only under `cfg(test)` and never ship.
    // ------------------------------------------------------------------

    /// Execute an arbitrary statement, surfacing any per-statement errors.
    #[cfg(test)]
    pub(crate) fn test_exec(&self, sql: &str) -> Result<()> {
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query(sql).await.context("test_exec query failed")
            })?;
            let errors = response.take_errors();
            if !errors.is_empty() {
                return Err(anyhow::anyhow!("test_exec statement errors: {:?}", errors));
            }
            Ok(())
        })
    }

    /// Read the RAW stored `chunk_count` for `kn-<id_part>` without the
    /// read-coalescing projection. Returns `None` when the field is NONE
    /// (i.e. the pre-backfill state) and `Some(n)` once it holds an int.
    #[cfg(test)]
    pub(crate) fn test_raw_chunk_count(&self, entry_id: &str) -> Result<Option<i64>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id).to_string();
        Self::runtime().block_on(async {
            let mut response = with_db!(self, db, {
                db.query("SELECT chunk_count FROM type::thing('knowledge', $id)")
                    .bind(("id", id_part.clone()))
                    .await
                    .context("test_raw_chunk_count query failed")
            })?;
            let rows: Vec<serde_json::Value> = response
                .take(0)
                .context("test_raw_chunk_count parse failed")?;
            let row = match rows.first() {
                Some(r) => r,
                None => return Ok(None),
            };
            // NONE serializes to JSON null; an int serializes to a number.
            match row.get("chunk_count") {
                Some(v) if v.is_null() => Ok(None),
                Some(v) => Ok(v.as_i64()),
                None => Ok(None),
            }
        })
    }
}
