use anyhow::{Context, Result};
use serde::Deserialize;
use surrealdb::sql::Thing;

use crate::types::Relationship;

use super::{ExistsRow, SurrealConnection, SurrealDatabase};

// =========================================================================
// RELATIONSHIP OPERATIONS
// =========================================================================

impl SurrealDatabase {
    /// Add a relationship between knowledge entries
    pub fn add_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<()> {
        Self::runtime().block_on(self.add_relationship_async(from, to, rel_type))
    }

    async fn add_relationship_async(&self, from: &str, to: &str, rel_type: &str) -> Result<()> {
        let from_id = from.strip_prefix("kn-").unwrap_or(from);
        let to_id = to.strip_prefix("kn-").unwrap_or(to);

        let from_thing = Thing::from(("knowledge", from_id));
        let to_thing = Thing::from(("knowledge", to_id));
        let rel_type_thing = Thing::from(("relationship_type", rel_type));

        with_db!(self, db, {
            db.query("RELATE $from->relates_to->$to SET relationship_type = $rel_type, created_at = time::now()")
                .bind(("from", from_thing))
                .bind(("to", to_thing))
                .bind(("rel_type", rel_type_thing))
                .await
                .context("Failed to create relationship")
        })?;

        Ok(())
    }

    /// List all relationships for a knowledge entry
    pub fn list_relationships(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        Self::runtime().block_on(self.list_relationships_async(entry_id))
    }

    async fn list_relationships_async(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        // Use meta::id() to extract plain string IDs from Thing record links.
        // Direct deserialization of Thing fields via serde_json::Value fails
        // because surrealdb::sql::Thing serializes as an untagged enum tuple
        // that serde_json cannot round-trip. meta::id() returns a plain string.
        #[derive(Debug, Deserialize)]
        struct RelRow {
            id: String,
            from_entry_id: String,
            to_entry_id: String,
            relationship_type: String,
            #[serde(default)]
            created_at: Option<String>,
        }

        let mut response = with_db!(self, db, {
            db.query(
                "SELECT meta::id(id) AS id,
                        meta::id(in) AS from_entry_id,
                        meta::id(out) AS to_entry_id,
                        meta::id(relationship_type) AS relationship_type,
                        <string>created_at AS created_at
                 FROM relates_to
                 WHERE in = $entry OR out = $entry
                 ORDER BY created_at DESC",
            )
            .bind(("entry", entry_thing))
            .await
            .context("Failed to query relationships")
        })?;

        let results: Vec<RelRow> = response.take(0)?;
        let relationships = results
            .into_iter()
            .map(|row| Relationship {
                id: row.id,
                from_entry_id: format!("kn-{}", row.from_entry_id),
                to_entry_id: format!("kn-{}", row.to_entry_id),
                relationship_type: row.relationship_type,
                created_at: row.created_at.unwrap_or_else(|| "unknown".to_string()),
            })
            .collect();

        Ok(relationships)
    }

    /// Delete a relationship by from/to/type triple
    pub fn delete_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_relationship_async(from, to, rel_type))
    }

    /// Delete a relationship edge by its record ID (e.g. "abc123" or the raw SurrealDB ID).
    pub fn delete_relationship_by_id(&self, id: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_relationship_by_id_async(id))
    }

    async fn delete_relationship_by_id_async(&self, id: &str) -> Result<bool> {
        // SurrealDB's RETURN BEFORE yields Thing-typed fields that serde_json cannot
        // round-trip. Instead: SELECT with meta::id() to check existence, then DELETE
        // without a RETURN clause (which is safe to take as Vec<Value> empty).
        //
        // NOTE: There is a TOCTOU window between the SELECT and DELETE — the edge
        // could be deleted by another caller between the two queries.  This is
        // acceptable for a single-user CLI tool where concurrent mutation is rare.
        let mut check = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id FROM relates_to WHERE meta::id(id) = $id LIMIT 1")
                .bind(("id", id.to_string()))
                .await
                .context("Failed to check relationship existence")
        })?;

        let exists: Vec<ExistsRow> = check.take(0)?;
        if exists.is_empty() {
            return Ok(false);
        }

        with_db!(self, db, {
            db.query("DELETE relates_to WHERE meta::id(id) = $id")
                .bind(("id", id.to_string()))
                .await
                .context("Failed to delete relationship by id")
        })?;

        Ok(true)
    }

    async fn delete_relationship_async(
        &self,
        from: &str,
        to: &str,
        rel_type: &str,
    ) -> Result<bool> {
        let from_id = from.strip_prefix("kn-").unwrap_or(from);
        let to_id = to.strip_prefix("kn-").unwrap_or(to);

        let from_thing = Thing::from(("knowledge", from_id));
        let to_thing = Thing::from(("knowledge", to_id));
        let rel_type_thing = Thing::from(("relationship_type", rel_type));

        // SurrealDB's RETURN BEFORE yields Thing-typed fields that serde_json cannot
        // round-trip. Check existence with meta::id() SELECT first, then DELETE without
        // a RETURN clause to avoid the deserialization error.
        let mut check = with_db!(self, db, {
            db.query(
                "SELECT meta::id(id) AS id FROM relates_to
                 WHERE in = $from AND out = $to AND relationship_type = $rel_type
                 LIMIT 1",
            )
            .bind(("from", from_thing.clone()))
            .bind(("to", to_thing.clone()))
            .bind(("rel_type", rel_type_thing.clone()))
            .await
            .context("Failed to check relationship existence")
        })?;

        let exists: Vec<ExistsRow> = check.take(0)?;
        if exists.is_empty() {
            return Ok(false);
        }

        with_db!(self, db, {
            db.query(
                "DELETE relates_to
                 WHERE in = $from AND out = $to AND relationship_type = $rel_type",
            )
            .bind(("from", from_thing))
            .bind(("to", to_thing))
            .bind(("rel_type", rel_type_thing))
            .await
            .context("Failed to delete relationship")
        })?;

        Ok(true)
    }

    /// Get facts extracted from a specific session
    pub fn get_facts_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        Self::runtime().block_on(self.get_facts_for_session_async(session_id))
    }

    async fn get_facts_for_session_async(&self, session_id: &str) -> Result<Vec<String>> {
        let session_id_normalized = session_id.strip_prefix("kn-").unwrap_or(session_id);
        let session_thing = Thing::from(("knowledge", session_id_normalized));

        let mut response = with_db!(self, db, {
            db.query(
                "SELECT VALUE meta::id(in) FROM relates_to
                 WHERE out = $session_id AND relationship_type = relationship_type:extracted_from",
            )
            .bind(("session_id", session_thing))
            .await
            .context("Failed to query facts for session")
        })?;

        let fact_ids: Vec<String> = response.take(0).unwrap_or_default();
        let facts_with_prefix: Vec<String> = fact_ids
            .into_iter()
            .map(|id| format!("kn-{}", id))
            .collect();

        Ok(facts_with_prefix)
    }

    /// Get the session a fact was extracted from
    pub fn get_session_for_fact(&self, fact_id: &str) -> Result<Option<String>> {
        Self::runtime().block_on(self.get_session_for_fact_async(fact_id))
    }

    async fn get_session_for_fact_async(&self, fact_id: &str) -> Result<Option<String>> {
        let fact_id_normalized = fact_id.strip_prefix("kn-").unwrap_or(fact_id);
        let fact_thing = Thing::from(("knowledge", fact_id_normalized));

        let mut response = with_db!(self, db, {
            db.query(
                "SELECT VALUE meta::id(out) FROM relates_to
                 WHERE in = $fact AND relationship_type = relationship_type:extracted_from",
            )
            .bind(("fact", fact_thing))
            .await
            .context("Failed to query session for fact")
        })?;

        let session_ids: Vec<String> = response.take(0).unwrap_or_default();

        Ok(session_ids.first().map(|id| format!("kn-{}", id)))
    }

    // =========================================================================
    // TAG OPERATIONS (not exposed in public API, handled via knowledge entry)
    // =========================================================================

    /// Get tags for an entry
    pub fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        Self::runtime().block_on(self.get_tags_for_entry_async(entry_id))
    }

    pub(super) async fn get_tags_for_entry_async(&self, entry_id: &str) -> Result<Vec<String>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        let mut tags_response = with_db!(self, db, {
            db.query("SELECT VALUE out.name FROM tagged_with WHERE in = $knowledge")
                .bind(("knowledge", entry_thing))
                .await
                .context("Failed to query tags")
        })?;

        let tags: Vec<String> = tags_response.take(0).unwrap_or_default();
        Ok(tags)
    }

    /// Set tags for an entry - handled automatically by upsert_knowledge
    pub fn set_tags_for_entry(&self, _entry_id: &str, _tags: &[String]) -> Result<()> {
        // Tags are managed via upsert_knowledge, this is a no-op for compatibility
        Ok(())
    }

    /// Get applicability for an entry
    pub fn get_applicability_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        Self::runtime().block_on(self.get_applicability_for_entry_async(entry_id))
    }

    pub(super) async fn get_applicability_for_entry_async(
        &self,
        entry_id: &str,
    ) -> Result<Vec<String>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        let mut app_response = with_db!(self, db, {
            db.query("SELECT VALUE meta::id(out) FROM applies_to WHERE in = $knowledge")
                .bind(("knowledge", entry_thing))
                .await
                .context("Failed to query applicability")
        })?;

        let applicability_raw: Vec<Thing> = app_response.take(0).unwrap_or_default();
        let applicability: Vec<String> = applicability_raw
            .into_iter()
            .map(|t| t.id.to_string())
            .collect();

        Ok(applicability)
    }

    /// Set applicability for an entry - handled automatically by upsert_knowledge
    pub fn set_applicability_for_entry(&self, _entry_id: &str, _ids: &[String]) -> Result<()> {
        // Applicability is managed via upsert_knowledge, this is a no-op for compatibility
        Ok(())
    }
}
