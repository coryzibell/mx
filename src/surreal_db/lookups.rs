use anyhow::{Context, Result};
use chrono::Utc;
use surrealdb::sql::Thing;

use crate::types::{
    Agent, ApplicabilityType, Category, ContentType, EntryType, Project, RelationshipType, Session,
    SessionType, SourceType,
};

use super::connection::normalize_datetime;
use super::{RecordId, SurrealConnection, SurrealDatabase, Tag};

// =========================================================================
// LOOKUP OPERATIONS
// =========================================================================

impl SurrealDatabase {
    /// List all categories
    pub fn list_categories(&self) -> Result<Vec<Category>> {
        Self::runtime().block_on(self.list_categories_async())
    }

    async fn list_categories_async(&self) -> Result<Vec<Category>> {
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM category ORDER BY id")
                .await
                .context("Failed to list categories")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, name, path, repo_url, description, active, <string>created_at AS created_at, <string>updated_at AS updated_at FROM project ORDER BY name")
                .await
                .context("Failed to list projects")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, domain, <string>created_at AS created_at, <string>updated_at AS updated_at FROM agent ORDER BY id")
                .await
                .context("Failed to list agents")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, name, <string>created_at AS created_at FROM tag ORDER BY name")
                .await
                .context("Failed to list tags")
        })?;

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

    /// List all distinct tag names, optionally filtered by category
    pub fn list_all_tags(&self, category: Option<&str>) -> Result<Vec<String>> {
        Self::runtime().block_on(self.list_all_tags_async(category.map(|s| s.to_string())))
    }

    async fn list_all_tags_async(&self, category: Option<String>) -> Result<Vec<String>> {
        let mut tags = if let Some(cat) = category {
            // Traverse from tag side: find tags whose knowledge entries belong to the category.
            // Filtering via `WHERE in.category = ...` on a graph edge table does not work in
            // SurrealDB 2.x — the predicate matches nothing even though the field is present.
            // Reverse traversal through the tag record works correctly.
            let mut response = with_db!(self, db, {
                db.query(
                    "SELECT VALUE name FROM tag \
                     WHERE <-tagged_with<-knowledge.category CONTAINS type::thing('category', $cat)",
                )
                .bind(("cat", cat))
                .await
                .context("Failed to list tags by category")
            })?;
            let tags: Vec<String> = response.take(0).unwrap_or_default();
            tags
        } else {
            // Only return tags that are actually in use (have at least one incoming edge).
            // The previous query used `array::distinct(out.name)` on the edge table, but
            // `out.name` is a scalar string per row — array::distinct expects an array and errors.
            let mut response = with_db!(self, db, {
                db.query("SELECT VALUE name FROM tag WHERE <-tagged_with")
                    .await
                    .context("Failed to list all tags")
            })?;
            let tags: Vec<String> = response.take(0).unwrap_or_default();
            tags
        };

        tags.sort();
        Ok(tags)
    }

    /// List all applicability types
    pub fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
        Self::runtime().block_on(self.list_applicability_types_async())
    }

    async fn list_applicability_types_async(&self) -> Result<Vec<ApplicabilityType>> {
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, scope, <string>created_at AS created_at FROM applicability_type ORDER BY id")
                .await
                .context("Failed to list applicability types")
        })?;

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

        let mut response = with_db!(self, db, {
            db.query(
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
            .context("Failed to upsert project")
        })?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        Ok(record_id)
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

        let mut response = with_db!(self, db, {
            db.query(
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
            .context("Failed to upsert applicability type")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM category WHERE id = type::thing('category', $id)")
                .bind(("id", id.to_string()))
                .await
                .context("Failed to query category")
        })?;

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

        let mut response = with_db!(self, db, {
            db.query(
                "UPSERT type::thing('category', $id) SET
                description = $description,
                created_at = <datetime>$created_at
            ",
            )
            .bind(("id", category.id.clone()))
            .bind(("description", category.description.clone()))
            .bind(("created_at", normalize_datetime(&created_at)))
            .await
            .context("Failed to upsert category")
        })?;

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
        let mut count_response = with_db!(self, db, {
            db.query("SELECT count() AS c FROM knowledge WHERE category = $category GROUP ALL")
                .bind(("category", category_thing.clone()))
                .await
                .context("Failed to count knowledge entries for category")
        })?;

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
        let result: Option<surrealdb::sql::Value> = with_db!(self, db, {
            db.delete(record_id.to_record_id())
                .await
                .context("Failed to delete category")
        })?;

        Ok(result.is_some())
    }

    /// Get project by ID
    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        Self::runtime().block_on(self.get_project_async(id))
    }

    async fn get_project_async(&self, id: &str) -> Result<Option<Project>> {
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, name, path, repo_url, description, active, <string>created_at AS created_at, <string>updated_at AS updated_at FROM project WHERE id = type::thing('project', $id)")
                .bind(("id", id.to_string()))
                .await
                .context("Failed to query project")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, domain, <string>created_at AS created_at, <string>updated_at AS updated_at FROM agent WHERE id = type::thing('agent', $id)")
                .bind(("id", id.to_string()))
                .await
                .context("Failed to query agent")
        })?;

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

        let mut response = with_db!(self, db, {
            db.query(
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
            .context("Failed to upsert agent")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM source_type ORDER BY id")
                .await
                .context("Failed to list source types")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM entry_type ORDER BY id")
                .await
                .context("Failed to list entry types")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, file_extensions, <string>created_at AS created_at FROM content_type ORDER BY id")
                .await
                .context("Failed to list content types")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, <string>created_at AS created_at FROM session_type ORDER BY id")
                .await
                .context("Failed to list session types")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query("SELECT meta::id(id) AS id, description, directional, <string>created_at AS created_at FROM relationship_type ORDER BY id")
                .await
                .context("Failed to list relationship types")
        })?;

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
