use anyhow::Result;
use std::path::Path;

use crate::db::{
    Agent, ApplicabilityType, Category, ContentType, EntryType, Project, Relationship,
    RelationshipType, Session, SessionType, SourceType,
};
use crate::knowledge::KnowledgeEntry;

/// Agent context for privacy-aware queries
#[derive(Debug, Clone)]
pub struct AgentContext {
    /// Current agent ID (None = anonymous/public-only access)
    pub agent_id: Option<String>,
    /// Whether to include private entries (requires matching agent_id)
    pub include_private: bool,
}

impl AgentContext {
    /// Public-only access (no private entries visible)
    pub fn public_only() -> Self {
        Self {
            agent_id: None,
            include_private: false,
        }
    }

    /// Agent with full access to their private entries
    pub fn for_agent(id: impl Into<String>) -> Self {
        Self {
            agent_id: Some(id.into()),
            include_private: true,
        }
    }

    /// Agent but only viewing public entries
    pub fn public_for_agent(id: impl Into<String>) -> Self {
        Self {
            agent_id: Some(id.into()),
            include_private: false,
        }
    }
}

/// Result of a wake-up cascade query
#[derive(Debug, Clone, serde::Serialize)]
pub struct WakeCascade {
    /// Layer 1: Foundational/transformative, resonance 8+
    pub core: Vec<crate::knowledge::KnowledgeEntry>,
    /// Layer 2: Last N days, sorted by resonance * recency
    pub recent: Vec<crate::knowledge::KnowledgeEntry>,
    /// Layer 3: Anchored to core/recent, resonance 5+
    pub bridges: Vec<crate::knowledge::KnowledgeEntry>,
}

impl WakeCascade {
    pub fn all_ids(&self) -> Vec<String> {
        self.core
            .iter()
            .chain(self.recent.iter())
            .chain(self.bridges.iter())
            .map(|e| e.id.clone())
            .collect()
    }
}

/// Abstract interface for knowledge storage backends (SQLite, SurrealDB, etc)
pub trait KnowledgeStore {
    // =========================================================================
    // KNOWLEDGE CRUD OPERATIONS
    // =========================================================================

    /// Upsert a knowledge entry (insert or update)
    fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()>;

    /// Get a knowledge entry by ID
    fn get(&self, id: &str, ctx: &AgentContext) -> Result<Option<KnowledgeEntry>>;

    /// Delete a knowledge entry
    fn delete(&self, id: &str) -> Result<bool>;

    /// Search knowledge entries
    fn search(&self, query: &str, ctx: &AgentContext) -> Result<Vec<KnowledgeEntry>>;

    /// List entries by category
    fn list_by_category(&self, category: &str, ctx: &AgentContext) -> Result<Vec<KnowledgeEntry>>;

    /// Count total entries
    fn count(&self) -> Result<usize>;

    /// Wake-up cascade query (three-layer resonance)
    fn wake_cascade(&self, ctx: &AgentContext, limit: usize, days: i64) -> Result<WakeCascade>;

    /// Update activation counts for loaded blooms
    fn update_activations(&self, ids: &[String]) -> Result<()>;

    // =========================================================================
    // TAG OPERATIONS
    // =========================================================================

    /// Get tags for an entry
    fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>>;

    /// Set tags for an entry (replaces all)
    fn set_tags_for_entry(&self, entry_id: &str, tags: &[String]) -> Result<()>;

    // =========================================================================
    // APPLICABILITY OPERATIONS
    // =========================================================================

    /// Get applicability for an entry
    fn get_applicability_for_entry(&self, entry_id: &str) -> Result<Vec<String>>;

    /// Set applicability for an entry (replaces all)
    fn set_applicability_for_entry(&self, entry_id: &str, ids: &[String]) -> Result<()>;

    /// List all applicability types
    fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>>;

    /// Upsert applicability type
    fn upsert_applicability_type(&self, atype: &ApplicabilityType) -> Result<()>;

    // =========================================================================
    // CATEGORY OPERATIONS
    // =========================================================================

    /// List all categories
    fn list_categories(&self) -> Result<Vec<Category>>;

    /// Get category by ID
    fn get_category(&self, id: &str) -> Result<Option<Category>>;

    /// Upsert category
    fn upsert_category(&self, category: &Category) -> Result<()>;

    /// Delete category (fails if entries use it)
    fn delete_category(&self, id: &str) -> Result<bool>;

    // =========================================================================
    // PROJECT OPERATIONS
    // =========================================================================

    /// List all projects
    fn list_projects(&self, active_only: bool) -> Result<Vec<Project>>;

    /// Get project by ID
    fn get_project(&self, id: &str) -> Result<Option<Project>>;

    /// Upsert project
    fn upsert_project(&self, project: &Project) -> Result<()>;

    /// Get tags for a project
    fn get_tags_for_project(&self, project_id: &str) -> Result<Vec<String>>;

    /// Set tags for a project
    fn set_tags_for_project(&self, project_id: &str, tags: &[String]) -> Result<()>;

    /// Get applicability for a project
    fn get_applicability_for_project(&self, project_id: &str) -> Result<Vec<String>>;

    /// Set applicability for a project
    fn set_applicability_for_project(&self, project_id: &str, ids: &[String]) -> Result<()>;

    // =========================================================================
    // AGENT OPERATIONS
    // =========================================================================

    /// List all agents
    fn list_agents(&self) -> Result<Vec<Agent>>;

    /// Get agent by ID
    fn get_agent(&self, id: &str) -> Result<Option<Agent>>;

    /// Upsert agent
    fn upsert_agent(&self, agent: &Agent) -> Result<()>;

    // =========================================================================
    // RELATIONSHIP OPERATIONS
    // =========================================================================

    /// List relationships for an entry
    fn list_relationships_for_entry(&self, entry_id: &str) -> Result<Vec<Relationship>>;

    /// Add relationship between entries
    fn add_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<String>;

    /// Delete relationship
    fn delete_relationship(&self, id: &str) -> Result<bool>;

    // =========================================================================
    // SESSION OPERATIONS
    // =========================================================================

    /// List sessions
    fn list_sessions(&self, project_id: Option<&str>) -> Result<Vec<Session>>;

    /// Get session by ID
    fn get_session(&self, id: &str) -> Result<Option<Session>>;

    /// Upsert session
    fn upsert_session(&self, session: &Session) -> Result<()>;

    // =========================================================================
    // TYPE LOOKUP OPERATIONS
    // =========================================================================

    /// List all source types
    fn list_source_types(&self) -> Result<Vec<SourceType>>;

    /// List all entry types
    fn list_entry_types(&self) -> Result<Vec<EntryType>>;

    /// List all content types
    fn list_content_types(&self) -> Result<Vec<ContentType>>;

    /// List all session types
    fn list_session_types(&self) -> Result<Vec<SessionType>>;

    /// List all relationship types
    fn list_relationship_types(&self) -> Result<Vec<RelationshipType>>;

    // =========================================================================
    // MIGRATION & INTROSPECTION
    // =========================================================================

    /// List tables (for migration status)
    fn list_tables(&self) -> Result<Vec<String>>;
}

/// Factory function to create appropriate store based on configuration
pub fn create_store(db_path: &Path) -> Result<Box<dyn KnowledgeStore>> {
    // Check environment variable first
    let backend = std::env::var("MX_MEMORY_BACKEND")
        .ok()
        .unwrap_or_else(|| "sqlite".to_string());

    match backend.as_str() {
        "surrealdb" | "surreal" => {
            // Replace .db extension with .surreal directory
            let surreal_path = db_path.with_extension("surreal");
            Ok(Box::new(crate::surreal_db::SurrealDatabase::open(
                surreal_path,
            )?))
        }
        _ => {
            // Default to SQLite
            Ok(Box::new(crate::db::Database::open(db_path)?))
        }
    }
}
