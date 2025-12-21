use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;
use surrealdb::RecordId as SurrealRecordId;
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb::sql::{Thing, Value};
use tokio::runtime::Runtime;

use crate::db::{
    Agent, ApplicabilityType, Category, ContentType, EntryType, Project, Relationship,
    RelationshipType, Session, SessionType, SourceType,
};
use crate::knowledge::KnowledgeEntry;
use crate::store::KnowledgeStore;

/// Embedded SurrealDB schema - applied on database open
const SCHEMA: &str = include_str!("../schema/surrealdb-schema.surql");

/// Tag record for SurrealDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// SurrealDB Thing wrapper for typed record IDs
#[derive(Debug, Clone)]
pub(crate) struct RecordId(Thing);

impl RecordId {
    fn new(table: &str, id: &str) -> Self {
        Self(Thing::from((table, id)))
    }

    fn as_thing(&self) -> &Thing {
        &self.0
    }

    fn into_thing(self) -> Thing {
        self.0
    }

    fn to_record_id(&self) -> SurrealRecordId {
        SurrealRecordId::from((self.0.tb.as_str(), self.0.id.to_string().as_str()))
    }
}

/// Normalize datetime string to RFC3339 format for SurrealDB
fn normalize_datetime(s: &str) -> String {
    // If already looks like RFC3339 (has T and timezone), return as-is
    if s.contains('T') && (s.ends_with('Z') || s.contains('+') || s.contains("-0")) {
        return s.to_string();
    }

    // SQLite format: "2025-11-29 08:10:33" -> "2025-11-29T08:10:33Z"
    if s.contains(' ') && !s.contains('T') {
        return s.replace(' ', "T") + "Z";
    }

    // Fallback: assume it's already good or add Z
    if !s.ends_with('Z') && !s.contains('+') {
        return format!("{}Z", s);
    }

    s.to_string()
}

/// SurrealDB-backed knowledge store
pub struct SurrealDatabase {
    db: Surreal<surrealdb::engine::local::Db>,
}

impl SurrealDatabase {
    /// Get or initialize the global tokio runtime
    fn runtime() -> &'static Runtime {
        static RT: OnceLock<Runtime> = OnceLock::new();
        RT.get_or_init(|| Runtime::new().expect("Failed to create tokio runtime"))
    }

    /// Open database at path, create if not exists, apply schema
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::runtime().block_on(Self::open_async(path))
    }

    async fn open_async<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create database directory: {:?}", parent))?;
        }

        // Connect to SurrealKv backend
        let db = Surreal::new::<SurrealKv>(path)
            .await
            .with_context(|| format!("Failed to open SurrealDB at {:?}", path))?;

        // Use namespace and database
        db.use_ns("memory")
            .use_db("knowledge")
            .await
            .context("Failed to set namespace and database")?;

        // Apply schema (idempotent)
        let mut response = db
            .query(SCHEMA)
            .await
            .context("Failed to apply database schema")?;

        // Check for errors - schema application returns multiple results
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Schema application failed: {:?}", errors));
        }

        Ok(Self { db })
    }

    /// Test helper - open temporary database
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        use tempfile::tempdir;

        let temp_dir = tempdir()?;
        Self::open(temp_dir.path())
    }

    /// Get reference to underlying Surreal instance
    pub fn inner(&self) -> &Surreal<surrealdb::engine::local::Db> {
        &self.db
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
            content_type = type::thing('content_type', $content_type_id)"
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

        // Bind required parameters
        let mut q = self
            .db
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
            ));

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

        // Execute the update
        let mut response = q.await.context("Failed to upsert knowledge record")?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        // Manage tags - delete old, create new
        let mut tag_delete_response = self
            .db
            .query("DELETE tagged_with WHERE in = $knowledge")
            .bind(("knowledge", record_id.0.clone()))
            .await
            .context("Failed to clear existing tags")?;

        let tag_delete_errors = tag_delete_response.take_errors();
        if !tag_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                tag_delete_errors
            ));
        }

        for tag_name in &entry.tags {
            // Ensure tag exists
            let tag_id = RecordId::new("tag", tag_name);
            let _: Option<Value> = self
                .db
                .update(tag_id.to_record_id())
                .content(serde_json::json!({
                    "name": tag_name
                }))
                .await
                .context("Failed to create tag")?;

            // Create edge
            let mut tag_edge_response = self
                .db
                .query("RELATE $knowledge->tagged_with->$tag")
                .bind(("knowledge", record_id.0.clone()))
                .bind(("tag", tag_id.0.clone()))
                .await
                .context("Failed to create tag edge")?;

            let tag_edge_errors = tag_edge_response.take_errors();
            if !tag_edge_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "SurrealDB returned errors: {:?}",
                    tag_edge_errors
                ));
            }
        }

        // Manage applicability - delete old, create new
        let mut app_delete_response = self
            .db
            .query("DELETE applies_to WHERE in = $knowledge")
            .bind(("knowledge", record_id.0.clone()))
            .await
            .context("Failed to clear existing applicability")?;

        let app_delete_errors = app_delete_response.take_errors();
        if !app_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                app_delete_errors
            ));
        }

        for app_type in &entry.applicability {
            let app_id = RecordId::new("applicability_type", app_type);
            let mut app_edge_response = self
                .db
                .query("RELATE $knowledge->applies_to->$app_type")
                .bind(("knowledge", record_id.0.clone()))
                .bind(("app_type", app_id.0.clone()))
                .await
                .context("Failed to create applicability edge")?;

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

        // Build visibility filter based on context
        // Use parameterized query for owner to prevent injection
        let (visibility_clause, current_agent) = if ctx.include_private {
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
        };

        let sql = format!(
            "SELECT
                meta::id(id) AS id, title, body, summary, file_path, content_hash, ephemeral,
                owner, visibility,
                meta::id(category) AS category_id,
                meta::id(source_type) AS source_type_id,
                meta::id(entry_type) AS entry_type_id,
                meta::id(content_type) AS content_type_id,
                IF source_project THEN meta::id(source_project) ELSE null END AS source_project_id,
                IF source_agent THEN meta::id(source_agent) ELSE null END AS source_agent_id,
                IF session THEN meta::id(session) ELSE null END AS session_id,
                <string>created_at AS created_at, <string>updated_at AS updated_at
            FROM knowledge
            WHERE meta::id(id) = $id {}",
            visibility_clause
        );

        let mut query = self.db.query(&sql).bind(("id", id_part.to_string()));
        if let Some(agent) = current_agent {
            query = query.bind(("current_agent", agent));
        }
        let mut response = query.await.context("Failed to query knowledge record")?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        if results.is_empty() {
            return Ok(None);
        }

        let obj = &results[0];
        self.value_to_knowledge_entry(obj.clone()).await.map(Some)
    }

    /// Delete a knowledge entry (edges cascade automatically)
    pub fn delete_knowledge(&self, id: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_knowledge_async(id))
    }

    async fn delete_knowledge_async(&self, id: &str) -> Result<bool> {
        let id_part = id.strip_prefix("kn-").unwrap_or(id);
        let record_id = RecordId::new("knowledge", id_part);

        let result: Option<Value> = self
            .db
            .delete(record_id.to_record_id())
            .await
            .context("Failed to delete knowledge record")?;

        Ok(result.is_some())
    }

    /// Search knowledge using BM25 full-text indexes
    pub fn search_knowledge(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.search_knowledge_async(query, ctx))
    }

    async fn search_knowledge_async(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        let query_owned = query.to_string();

        // Build visibility filter based on context
        // Use parameterized query for owner to prevent injection
        let (visibility_clause, current_agent) = if ctx.include_private {
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
        };

        let sql = format!(
            "SELECT
                meta::id(id) AS id, title, body, summary, file_path, content_hash, ephemeral,
                owner, visibility,
                meta::id(category) AS category_id,
                meta::id(source_type) AS source_type_id,
                meta::id(entry_type) AS entry_type_id,
                meta::id(content_type) AS content_type_id,
                IF source_project THEN meta::id(source_project) ELSE null END AS source_project_id,
                IF source_agent THEN meta::id(source_agent) ELSE null END AS source_agent_id,
                IF session THEN meta::id(session) ELSE null END AS session_id,
                <string>created_at AS created_at, <string>updated_at AS updated_at
            FROM knowledge
            WHERE (title @@ $query OR body @@ $query OR summary @@ $query) {}",
            visibility_clause
        );

        let mut query_builder = self.db.query(&sql).bind(("query", query_owned));
        if let Some(agent) = current_agent {
            query_builder = query_builder.bind(("current_agent", agent));
        }
        let mut response = query_builder
            .await
            .context("Failed to execute search query")?;

        let results: Vec<serde_json::Value> =
            response.take(0).context("Failed to parse search results")?;

        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Helper: Convert SurrealDB query result to KnowledgeEntry
    async fn value_to_knowledge_entry(&self, obj: serde_json::Value) -> Result<KnowledgeEntry> {
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
        let mut tags_response = self
            .db
            .query("SELECT VALUE ->tagged_with->tag.name FROM $knowledge")
            .bind(("knowledge", knowledge_thing.clone()))
            .await
            .context("Failed to query tags")?;
        let tags: Vec<String> = tags_response.take(0).unwrap_or_default();

        // Fetch applicability
        let mut app_response = self
            .db
            .query("SELECT VALUE ->applies_to->applicability_type.id FROM $knowledge")
            .bind(("knowledge", knowledge_thing))
            .await
            .context("Failed to query applicability")?;
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
        })
    }

    // =========================================================================
    // LOOKUP OPERATIONS
    // =========================================================================

    /// List all categories
    pub fn list_categories(&self) -> Result<Vec<Category>> {
        Self::runtime().block_on(self.list_categories_async())
    }

    async fn list_categories_async(&self) -> Result<Vec<Category>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM category ORDER BY id")
            .await
            .context("Failed to list categories")?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        let mut categories = Vec::new();
        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            categories.push(Category {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(categories)
    }

    /// List all projects
    pub fn list_projects(&self) -> Result<Vec<Project>> {
        Self::runtime().block_on(self.list_projects_async())
    }

    async fn list_projects_async(&self) -> Result<Vec<Project>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, name, path, repo_url, description, active, <string>created_at AS created_at, <string>updated_at AS updated_at FROM project ORDER BY name")
            .await
            .context("Failed to list projects")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut projects = Vec::new();

        for obj in results {
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            projects.push(Project {
                id,
                name: obj["name"].as_str().unwrap_or_default().to_string(),
                path: obj["path"].as_str().map(|s| s.to_string()),
                repo_url: obj["repo_url"].as_str().map(|s| s.to_string()),
                description: obj["description"].as_str().map(|s| s.to_string()),
                active: obj["active"].as_bool().unwrap_or(true),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
                updated_at: obj["updated_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(projects)
    }

    /// List all agents
    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        Self::runtime().block_on(self.list_agents_async())
    }

    async fn list_agents_async(&self) -> Result<Vec<Agent>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, domain, <string>created_at AS created_at, <string>updated_at AS updated_at FROM agent ORDER BY id")
            .await
            .context("Failed to list agents")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut agents = Vec::new();

        for obj in results {
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            agents.push(Agent {
                id,
                description: obj["description"].as_str().map(|s| s.to_string()),
                domain: obj["domain"].as_str().map(|s| s.to_string()),
                created_at: obj["created_at"].as_str().map(|s| s.to_string()),
                updated_at: obj["updated_at"].as_str().map(|s| s.to_string()),
            });
        }

        Ok(agents)
    }

    /// List all tags
    pub fn list_tags(&self) -> Result<Vec<Tag>> {
        Self::runtime().block_on(self.list_tags_async())
    }

    async fn list_tags_async(&self) -> Result<Vec<Tag>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, name, <string>created_at AS created_at FROM tag ORDER BY name")
            .await
            .context("Failed to list tags")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut tags = Vec::new();

        for obj in results {
            tags.push(Tag {
                name: obj["name"].as_str().unwrap_or_default().to_string(),
                created_at: obj["created_at"].as_str().map(|s| s.to_string()),
            });
        }

        Ok(tags)
    }

    /// List all applicability types
    pub fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
        Self::runtime().block_on(self.list_applicability_types_async())
    }

    async fn list_applicability_types_async(&self) -> Result<Vec<ApplicabilityType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, scope, <string>created_at AS created_at FROM applicability_type ORDER BY id")
            .await
            .context("Failed to list applicability types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            types.push(ApplicabilityType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                scope: obj["scope"].as_str().map(|s| s.to_string()),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }

    /// Upsert a project (returns RecordId)
    pub fn upsert_project_internal(&self, project: &Project) -> Result<RecordId> {
        Self::runtime().block_on(self.upsert_project_async(project))
    }

    async fn upsert_project_async(&self, project: &Project) -> Result<RecordId> {
        let record_id = RecordId::new("project", &project.id);

        // Always include datetimes - use current time if not provided
        let now = Utc::now().to_rfc3339();
        let created_at = if project.created_at.is_empty() {
            now.clone()
        } else {
            project.created_at.clone()
        };
        let updated_at = if project.updated_at.is_empty() {
            now.clone()
        } else {
            project.updated_at.clone()
        };

        let mut response = self
            .db
            .query(
                "UPSERT type::thing('project', $id) SET
                name = $name,
                path = $path,
                repo_url = $repo_url,
                description = $description,
                active = $active,
                created_at = <datetime>$created_at,
                updated_at = <datetime>$updated_at
            ",
            )
            .bind(("id", project.id.clone()))
            .bind(("name", project.name.clone()))
            .bind(("path", project.path.clone()))
            .bind(("repo_url", project.repo_url.clone()))
            .bind(("description", project.description.clone()))
            .bind(("active", project.active))
            .bind(("created_at", normalize_datetime(&created_at)))
            .bind(("updated_at", normalize_datetime(&updated_at)))
            .await
            .context("Failed to upsert project")?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        Ok(record_id)
    }

    // =========================================================================
    // RELATIONSHIP OPERATIONS
    // =========================================================================

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

        self.db
            .query("RELATE $from->relates_to->$to SET relationship_type = $rel_type")
            .bind(("from", from_thing))
            .bind(("to", to_thing))
            .bind(("rel_type", rel_type_thing))
            .await
            .context("Failed to create relationship")?;

        Ok(())
    }

    /// List all relationships for a knowledge entry
    pub fn list_relationships(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        Self::runtime().block_on(self.list_relationships_async(entry_id))
    }

    async fn list_relationships_async(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        // Query both outgoing and incoming relationships
        let mut response = self.db
            .query(
                "SELECT id, in AS from_entry_id, out AS to_entry_id, relationship_type, <string>created_at AS created_at
                 FROM relates_to
                 WHERE in = $entry OR out = $entry
                 ORDER BY created_at DESC"
            )
            .bind(("entry", entry_thing))
            .await
            .context("Failed to query relationships")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut relationships = Vec::new();

        for obj in results {
            let from_thing: Thing = serde_json::from_value(obj["from_entry_id"].clone())?;
            let to_thing: Thing = serde_json::from_value(obj["to_entry_id"].clone())?;
            let rel_type_thing: Thing = serde_json::from_value(obj["relationship_type"].clone())?;
            let id_thing: Thing = serde_json::from_value(obj["id"].clone())?;

            relationships.push(Relationship {
                id: id_thing.id.to_string(),
                from_entry_id: format!("kn-{}", from_thing.id),
                to_entry_id: format!("kn-{}", to_thing.id),
                relationship_type: rel_type_thing.id.to_string(),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(relationships)
    }

    /// Delete a relationship by from/to/type triple
    pub fn delete_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_relationship_async(from, to, rel_type))
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

        let mut response = self
            .db
            .query(
                "DELETE relates_to
                 WHERE in = $from AND out = $to AND relationship_type = $rel_type
                 RETURN BEFORE",
            )
            .bind(("from", from_thing))
            .bind(("to", to_thing))
            .bind(("rel_type", rel_type_thing))
            .await
            .context("Failed to delete relationship")?;

        let deleted: Vec<Value> = response.take(0)?;
        Ok(!deleted.is_empty())
    }

    // =========================================================================
    // TAG OPERATIONS (not exposed in public API, handled via knowledge entry)
    // =========================================================================

    /// Get tags for an entry
    pub fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        Self::runtime().block_on(self.get_tags_for_entry_async(entry_id))
    }

    async fn get_tags_for_entry_async(&self, entry_id: &str) -> Result<Vec<String>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        let mut tags_response = self
            .db
            .query("SELECT VALUE ->tagged_with->tag.name FROM $knowledge")
            .bind(("knowledge", entry_thing))
            .await
            .context("Failed to query tags")?;

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

    async fn get_applicability_for_entry_async(&self, entry_id: &str) -> Result<Vec<String>> {
        let id_part = entry_id.strip_prefix("kn-").unwrap_or(entry_id);
        let entry_thing = Thing::from(("knowledge", id_part));

        let mut app_response = self
            .db
            .query("SELECT VALUE ->applies_to->applicability_type.id FROM $knowledge")
            .bind(("knowledge", entry_thing))
            .await
            .context("Failed to query applicability")?;

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

    /// Upsert applicability type
    pub fn upsert_applicability_type(&self, atype: &ApplicabilityType) -> Result<()> {
        Self::runtime().block_on(self.upsert_applicability_type_async(atype))
    }

    async fn upsert_applicability_type_async(&self, atype: &ApplicabilityType) -> Result<()> {
        // Always include datetimes - use current time if not provided
        let now = Utc::now().to_rfc3339();
        let created_at = if atype.created_at.is_empty() {
            now
        } else {
            atype.created_at.clone()
        };

        let mut response = self
            .db
            .query(
                "UPSERT type::thing('applicability_type', $id) SET
                description = $description,
                scope = $scope,
                created_at = <datetime>$created_at
            ",
            )
            .bind(("id", atype.id.clone()))
            .bind(("description", atype.description.clone()))
            .bind(("scope", atype.scope.clone()))
            .bind(("created_at", normalize_datetime(&created_at)))
            .await
            .context("Failed to upsert applicability type")?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        Ok(())
    }

    /// Get category by ID
    pub fn get_category(&self, id: &str) -> Result<Option<Category>> {
        Self::runtime().block_on(self.get_category_async(id))
    }

    async fn get_category_async(&self, id: &str) -> Result<Option<Category>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM category WHERE id = type::thing('category', $id)")
            .bind(("id", id.to_string()))
            .await
            .context("Failed to query category")?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        if results.is_empty() {
            return Ok(None);
        }

        let obj = &results[0];
        let id_str = obj["id"].as_str().unwrap_or_default().to_string();

        Ok(Some(Category {
            id: id_str,
            description: obj["description"].as_str().unwrap_or_default().to_string(),
            created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
        }))
    }

    /// Upsert a category
    pub fn upsert_category(&self, category: &Category) -> Result<()> {
        Self::runtime().block_on(self.upsert_category_async(category))
    }

    async fn upsert_category_async(&self, category: &Category) -> Result<()> {
        // Always include datetime - use current time if not provided
        let now = Utc::now().to_rfc3339();
        let created_at = if category.created_at.is_empty() {
            now
        } else {
            category.created_at.clone()
        };

        let mut response = self
            .db
            .query(
                "UPSERT type::thing('category', $id) SET
                description = $description,
                created_at = <datetime>$created_at
            ",
            )
            .bind(("id", category.id.clone()))
            .bind(("description", category.description.clone()))
            .bind(("created_at", normalize_datetime(&created_at)))
            .await
            .context("Failed to upsert category")?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        Ok(())
    }

    /// Delete a category (only if no entries use it)
    pub fn delete_category(&self, id: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_category_async(id))
    }

    async fn delete_category_async(&self, id: &str) -> Result<bool> {
        let category_thing = Thing::from(("category", id));

        // Check if any knowledge entries use this category
        let mut count_response = self
            .db
            .query("SELECT count() AS c FROM knowledge WHERE category = $category GROUP ALL")
            .bind(("category", category_thing.clone()))
            .await
            .context("Failed to count knowledge entries for category")?;

        let count_results: Vec<serde_json::Value> = count_response.take(0)?;
        let count = count_results
            .first()
            .and_then(|v| v["c"].as_i64())
            .unwrap_or(0);

        if count > 0 {
            return Err(anyhow::anyhow!(
                "Cannot remove category '{}': {} entries still use it",
                id,
                count
            ));
        }

        // Delete the category
        let record_id = RecordId::new("category", id);
        let result: Option<Value> = self
            .db
            .delete(record_id.to_record_id())
            .await
            .context("Failed to delete category")?;

        Ok(result.is_some())
    }

    /// Get project by ID
    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        Self::runtime().block_on(self.get_project_async(id))
    }

    async fn get_project_async(&self, id: &str) -> Result<Option<Project>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, name, path, repo_url, description, active, <string>created_at AS created_at, <string>updated_at AS updated_at FROM project WHERE id = type::thing('project', $id)")
            .bind(("id", id.to_string()))
            .await
            .context("Failed to query project")?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        if results.is_empty() {
            return Ok(None);
        }

        let obj = &results[0];
        let id_str = obj["id"].as_str().unwrap_or_default().to_string();

        Ok(Some(Project {
            id: id_str,
            name: obj["name"].as_str().unwrap_or_default().to_string(),
            path: obj["path"].as_str().map(|s| s.to_string()),
            repo_url: obj["repo_url"].as_str().map(|s| s.to_string()),
            description: obj["description"].as_str().map(|s| s.to_string()),
            active: obj["active"].as_bool().unwrap_or(true),
            created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            updated_at: obj["updated_at"].as_str().unwrap_or_default().to_string(),
        }))
    }

    /// Get agent by ID
    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        Self::runtime().block_on(self.get_agent_async(id))
    }

    async fn get_agent_async(&self, id: &str) -> Result<Option<Agent>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, domain, <string>created_at AS created_at, <string>updated_at AS updated_at FROM agent WHERE id = type::thing('agent', $id)")
            .bind(("id", id.to_string()))
            .await
            .context("Failed to query agent")?;

        let results: Vec<serde_json::Value> = response.take(0)?;

        if results.is_empty() {
            return Ok(None);
        }

        let obj = &results[0];
        let id_str = obj["id"].as_str().unwrap_or_default().to_string();

        Ok(Some(Agent {
            id: id_str,
            description: obj["description"].as_str().map(|s| s.to_string()),
            domain: obj["domain"].as_str().map(|s| s.to_string()),
            created_at: obj["created_at"].as_str().map(|s| s.to_string()),
            updated_at: obj["updated_at"].as_str().map(|s| s.to_string()),
        }))
    }

    /// Upsert agent
    pub fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        Self::runtime().block_on(self.upsert_agent_async(agent))
    }

    async fn upsert_agent_async(&self, agent: &Agent) -> Result<()> {
        // Always include datetimes - use current time if not provided
        let now = Utc::now().to_rfc3339();
        let created_at = agent.created_at.clone().unwrap_or_else(|| now.clone());
        let updated_at = agent.updated_at.clone().unwrap_or_else(|| now.clone());

        let mut response = self
            .db
            .query(
                "UPSERT type::thing('agent', $id) SET
                description = $description,
                domain = $domain,
                created_at = <datetime>$created_at,
                updated_at = <datetime>$updated_at
            ",
            )
            .bind(("id", agent.id.clone()))
            .bind(("description", agent.description.clone()))
            .bind(("domain", agent.domain.clone()))
            .bind(("created_at", normalize_datetime(&created_at)))
            .bind(("updated_at", normalize_datetime(&updated_at)))
            .await
            .context("Failed to upsert agent")?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        Ok(())
    }

    /// Get tags for a project
    pub fn get_tags_for_project(&self, _project_id: &str) -> Result<Vec<String>> {
        // Not implemented in SurrealDB schema yet
        Ok(vec![])
    }

    /// Set tags for a project
    pub fn set_tags_for_project(&self, _project_id: &str, _tags: &[String]) -> Result<()> {
        // Not implemented in SurrealDB schema yet
        Ok(())
    }

    /// Get applicability for a project
    pub fn get_applicability_for_project(&self, _project_id: &str) -> Result<Vec<String>> {
        // Not implemented in SurrealDB schema yet
        Ok(vec![])
    }

    /// Set applicability for a project
    pub fn set_applicability_for_project(&self, _project_id: &str, _ids: &[String]) -> Result<()> {
        // Not implemented in SurrealDB schema yet
        Ok(())
    }

    /// List tables - SurrealDB uses tables, return table names
    pub fn list_tables(&self) -> Result<Vec<String>> {
        Self::runtime().block_on(self.list_tables_async())
    }

    async fn list_tables_async(&self) -> Result<Vec<String>> {
        let mut response = self
            .db
            .query("INFO FOR DB")
            .await
            .context("Failed to query database info")?;

        // SurrealDB INFO returns complex metadata - convert to JSON
        let info: Option<Value> = response.take(0)?;
        let mut tables = Vec::new();

        if let Some(info_value) = info {
            let info_json = info_value.into_json();
            if let Some(tables_obj) = info_json.get("tables").and_then(|v| v.as_object()) {
                for table_name in tables_obj.keys() {
                    tables.push(table_name.clone());
                }
                tables.sort();
            }
        }

        Ok(tables)
    }

    /// Count total knowledge entries
    pub fn count(&self) -> Result<usize> {
        Self::runtime().block_on(self.count_async())
    }

    async fn count_async(&self) -> Result<usize> {
        let mut response = self
            .db
            .query("SELECT count() AS c FROM knowledge GROUP ALL")
            .await
            .context("Failed to count knowledge entries")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let count = results.first().and_then(|v| v["c"].as_i64()).unwrap_or(0) as usize;
        Ok(count)
    }

    /// List entries by category
    pub fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_by_category_async(category, ctx))
    }

    async fn list_by_category_async(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        let category_thing = Thing::from(("category", category));

        // Build visibility filter based on context
        // Use parameterized query for owner to prevent injection
        let (visibility_clause, current_agent) = if ctx.include_private {
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
        };

        let sql = format!(
            "SELECT
                meta::id(id) AS id, title, body, summary, file_path, content_hash, ephemeral,
                owner, visibility,
                meta::id(category) AS category_id,
                meta::id(source_type) AS source_type_id,
                meta::id(entry_type) AS entry_type_id,
                meta::id(content_type) AS content_type_id,
                IF source_project THEN meta::id(source_project) ELSE null END AS source_project_id,
                IF source_agent THEN meta::id(source_agent) ELSE null END AS source_agent_id,
                IF session THEN meta::id(session) ELSE null END AS session_id,
                <string>created_at AS created_at, <string>updated_at AS updated_at
            FROM knowledge
            WHERE category = $category {}
            ORDER BY title",
            visibility_clause
        );

        let mut query = self.db.query(&sql).bind(("category", category_thing));
        if let Some(agent) = current_agent {
            query = query.bind(("current_agent", agent));
        }
        let mut response = query
            .await
            .context("Failed to query knowledge by category")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();

        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// List sessions
    pub fn list_sessions(&self, _project_id: Option<&str>) -> Result<Vec<Session>> {
        // Not fully implemented yet - return empty
        Ok(vec![])
    }

    /// Get session by ID
    pub fn get_session(&self, _id: &str) -> Result<Option<Session>> {
        // Not fully implemented yet
        Ok(None)
    }

    /// Upsert session
    pub fn upsert_session(&self, _session: &Session) -> Result<()> {
        // Not fully implemented yet
        Ok(())
    }

    /// List source types
    pub fn list_source_types(&self) -> Result<Vec<SourceType>> {
        Self::runtime().block_on(self.list_source_types_async())
    }

    async fn list_source_types_async(&self) -> Result<Vec<SourceType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM source_type ORDER BY id")
            .await
            .context("Failed to list source types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            types.push(SourceType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }

    /// List entry types
    pub fn list_entry_types(&self) -> Result<Vec<EntryType>> {
        Self::runtime().block_on(self.list_entry_types_async())
    }

    async fn list_entry_types_async(&self) -> Result<Vec<EntryType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM entry_type ORDER BY id")
            .await
            .context("Failed to list entry types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            types.push(EntryType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }

    /// List content types
    pub fn list_content_types(&self) -> Result<Vec<ContentType>> {
        Self::runtime().block_on(self.list_content_types_async())
    }

    async fn list_content_types_async(&self) -> Result<Vec<ContentType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, file_extensions, <string>created_at AS created_at FROM content_type ORDER BY id")
            .await
            .context("Failed to list content types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            // Parse array of file extensions
            let file_extensions = obj["file_extensions"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });

            types.push(ContentType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                file_extensions,
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }

    /// List session types
    pub fn list_session_types(&self) -> Result<Vec<SessionType>> {
        Self::runtime().block_on(self.list_session_types_async())
    }

    async fn list_session_types_async(&self) -> Result<Vec<SessionType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM session_type ORDER BY id")
            .await
            .context("Failed to list session types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            types.push(SessionType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }

    /// List relationship types
    pub fn list_relationship_types(&self) -> Result<Vec<RelationshipType>> {
        Self::runtime().block_on(self.list_relationship_types_async())
    }

    async fn list_relationship_types_async(&self) -> Result<Vec<RelationshipType>> {
        let mut response = self.db
            .query("SELECT meta::id(id) AS id, description, directional, <string>created_at AS created_at FROM relationship_type ORDER BY id")
            .await
            .context("Failed to list relationship types")?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut types = Vec::new();

        for obj in results {
            // Parse string from id field
            let id = obj["id"].as_str().unwrap_or_default().to_string();

            types.push(RelationshipType {
                id,
                description: obj["description"].as_str().unwrap_or_default().to_string(),
                directional: obj["directional"].as_bool().unwrap_or(false),
                created_at: obj["created_at"].as_str().unwrap_or_default().to_string(),
            });
        }

        Ok(types)
    }
}

// ============================================================================
// KNOWLEDGESTORE TRAIT IMPLEMENTATION
// ============================================================================

impl KnowledgeStore for SurrealDatabase {
    fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.upsert_knowledge_internal(entry)?;
        Ok(())
    }

    fn get(&self, id: &str, ctx: &crate::store::AgentContext) -> Result<Option<KnowledgeEntry>> {
        self.get_knowledge(id, ctx)
    }

    fn delete(&self, id: &str) -> Result<bool> {
        self.delete_knowledge(id)
    }

    fn search(&self, query: &str, ctx: &crate::store::AgentContext) -> Result<Vec<KnowledgeEntry>> {
        self.search_knowledge(query, ctx)
    }

    fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.list_by_category(category, ctx)
    }

    fn count(&self) -> Result<usize> {
        self.count()
    }

    fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        self.get_tags_for_entry(entry_id)
    }

    fn set_tags_for_entry(&self, entry_id: &str, tags: &[String]) -> Result<()> {
        self.set_tags_for_entry(entry_id, tags)
    }

    fn get_applicability_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        self.get_applicability_for_entry(entry_id)
    }

    fn set_applicability_for_entry(&self, entry_id: &str, ids: &[String]) -> Result<()> {
        self.set_applicability_for_entry(entry_id, ids)
    }

    fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
        self.list_applicability_types()
    }

    fn upsert_applicability_type(&self, atype: &ApplicabilityType) -> Result<()> {
        self.upsert_applicability_type(atype)
    }

    fn list_categories(&self) -> Result<Vec<Category>> {
        self.list_categories()
    }

    fn get_category(&self, id: &str) -> Result<Option<Category>> {
        self.get_category(id)
    }

    fn upsert_category(&self, category: &Category) -> Result<()> {
        self.upsert_category(category)
    }

    fn delete_category(&self, id: &str) -> Result<bool> {
        self.delete_category(id)
    }

    fn list_projects(&self, _active_only: bool) -> Result<Vec<Project>> {
        // SurrealDB implementation doesn't filter by active yet
        self.list_projects()
    }

    fn get_project(&self, id: &str) -> Result<Option<Project>> {
        self.get_project(id)
    }

    fn upsert_project(&self, project: &Project) -> Result<()> {
        self.upsert_project_internal(project)?;
        Ok(())
    }

    fn get_tags_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        self.get_tags_for_project(project_id)
    }

    fn set_tags_for_project(&self, project_id: &str, tags: &[String]) -> Result<()> {
        self.set_tags_for_project(project_id, tags)
    }

    fn get_applicability_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        self.get_applicability_for_project(project_id)
    }

    fn set_applicability_for_project(&self, project_id: &str, ids: &[String]) -> Result<()> {
        self.set_applicability_for_project(project_id, ids)
    }

    fn list_agents(&self) -> Result<Vec<Agent>> {
        self.list_agents()
    }

    fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        self.get_agent(id)
    }

    fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        self.upsert_agent(agent)
    }

    fn list_relationships_for_entry(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        self.list_relationships(entry_id)
    }

    fn add_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<String> {
        self.add_relationship(from, to, rel_type)?;
        // Return a synthetic ID since SurrealDB edge records don't have simple IDs
        Ok(format!("rel-{}-{}", from, to))
    }

    fn delete_relationship(&self, id: &str) -> Result<bool> {
        // SurrealDB delete_relationship takes from/to/type triple, not ID
        // For now, return false - this method isn't used in current CLI
        let _ = id;
        Ok(false)
    }

    fn list_tables(&self) -> Result<Vec<String>> {
        self.list_tables()
    }

    fn list_sessions(&self, project_id: Option<&str>) -> Result<Vec<Session>> {
        self.list_sessions(project_id)
    }

    fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.get_session(id)
    }

    fn upsert_session(&self, session: &Session) -> Result<()> {
        self.upsert_session(session)
    }

    fn list_source_types(&self) -> Result<Vec<SourceType>> {
        self.list_source_types()
    }

    fn list_entry_types(&self) -> Result<Vec<EntryType>> {
        self.list_entry_types()
    }

    fn list_content_types(&self) -> Result<Vec<ContentType>> {
        self.list_content_types()
    }

    fn list_session_types(&self) -> Result<Vec<SessionType>> {
        self.list_session_types()
    }

    fn list_relationship_types(&self) -> Result<Vec<RelationshipType>> {
        self.list_relationship_types()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        // Test that database opens without error
        let _db = SurrealDatabase::open_in_memory().unwrap();
    }

    #[test]
    fn test_schema_applies_without_error() {
        // Opening applies schema - if this succeeds, schema is valid
        let _db = SurrealDatabase::open_in_memory().unwrap();
    }

    #[test]
    fn test_open_with_path() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.surreal");

        // Open database at specific path
        let _db = SurrealDatabase::open(&db_path).unwrap();

        // Verify directory was created
        assert!(db_path.exists());
        assert!(db_path.is_dir());
    }

    #[test]
    fn test_upsert_applicability_type_with_datetime() {
        use crate::db::ApplicabilityType;

        let db = SurrealDatabase::open_in_memory().unwrap();

        // Create an applicability type with RFC3339 datetime
        let atype = ApplicabilityType {
            id: "test_type".to_string(),
            description: "Test applicability type".to_string(),
            scope: Some("test".to_string()),
            created_at: "2025-11-29T12:00:00Z".to_string(),
        };

        // Upsert should succeed without datetime parsing errors
        // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
        db.upsert_applicability_type(&atype).unwrap();
    }

    #[test]
    fn test_upsert_project_with_datetime() {
        use crate::db::Project;

        let db = SurrealDatabase::open_in_memory().unwrap();

        // Create a project with RFC3339 datetimes
        let project = Project {
            id: "test_project".to_string(),
            name: "Test Project".to_string(),
            path: Some("/test/path".to_string()),
            repo_url: None,
            description: Some("Test description".to_string()),
            active: true,
            created_at: "2025-11-29T12:00:00Z".to_string(),
            updated_at: "2025-11-29T12:30:00Z".to_string(),
        };

        // Upsert should succeed without datetime parsing errors
        // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
        db.upsert_project(&project).unwrap();
    }

    #[test]
    fn test_upsert_agent_with_datetime() {
        use crate::db::Agent;

        let db = SurrealDatabase::open_in_memory().unwrap();

        // Create an agent with RFC3339 datetimes
        let agent = Agent {
            id: "test_agent".to_string(),
            description: Some("Test agent".to_string()),
            domain: Some("testing".to_string()),
            created_at: Some("2025-11-29T12:00:00Z".to_string()),
            updated_at: Some("2025-11-29T12:30:00Z".to_string()),
        };

        // Upsert should succeed without datetime parsing errors
        // This was previously failing with: "Found '2025-11-29T...' for field `created_at`, but expected a datetime"
        db.upsert_agent(&agent).unwrap();
    }
}
