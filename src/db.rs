use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::knowledge::KnowledgeEntry;
use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: i32 = 3;

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub description: Option<String>,
    pub domain: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub repo_url: Option<String>,
    pub description: Option<String>,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicabilityType {
    pub id: String,
    pub description: String,
    pub scope: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceType {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryType {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipType {
    pub id: String,
    pub description: String,
    pub directional: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionType {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub session_type_id: String,
    pub project_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metadata: Option<String>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {:?}", path))?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        // Check schema version
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);

        match version {
            0..=1 => {
                // Fresh install - apply full v3 schema
                self.conn.execute_batch(include_str!("schema.sql"))?;
                self.conn.execute("PRAGMA user_version = 3", [])?;
            }
            2 => {
                // Migrate from v2 to v3
                eprintln!("Migrating Zion schema from v2 to v3...");
                self.conn
                    .execute_batch(include_str!("migrations/v2_to_v3.sql"))?;
                eprintln!("Migration complete.");
            }
            3 => {
                // Current version
            }
            _ => {
                anyhow::bail!("Unknown schema version: {}", version);
            }
        }

        Ok(())
    }

    pub fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO knowledge (id, category_id, title, body, summary,
                                   source_project_id, source_agent_id, file_path,
                                   created_at, updated_at, content_hash,
                                   source_type_id, entry_type_id, session_id, ephemeral)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(id) DO UPDATE SET
                category_id = excluded.category_id,
                title = excluded.title,
                body = excluded.body,
                summary = excluded.summary,
                source_project_id = excluded.source_project_id,
                source_agent_id = excluded.source_agent_id,
                file_path = excluded.file_path,
                updated_at = excluded.updated_at,
                content_hash = excluded.content_hash,
                source_type_id = excluded.source_type_id,
                entry_type_id = excluded.entry_type_id,
                session_id = excluded.session_id,
                ephemeral = excluded.ephemeral
            "#,
            params![
                entry.id,
                entry.category,
                entry.title,
                entry.body,
                entry.summary,
                entry.source_project,
                entry.source_agent,
                entry.file_path,
                entry.created_at,
                entry.updated_at,
                entry.content_hash,
                entry.source_type,
                entry.entry_type,
                entry.session_id,
                entry.ephemeral,
            ],
        )?;

        // Update tags junction table
        self.set_tags_for_entry(&entry.id, &entry.tags)?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<KnowledgeEntry>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral
            FROM knowledge
            WHERE title LIKE ?1 OR body LIKE ?1 OR summary LIKE ?1
            ORDER BY updated_at DESC
            "#,
        )?;

        let mut entries = stmt
            .query_map(params![pattern], |row| {
                let id: String = row.get(0)?;
                Ok(KnowledgeEntry {
                    id: id.clone(),
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: None,
                    source_project: row.get(5)?,
                    source_agent: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type: row.get(11)?,
                    entry_type: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load tags for each entry
        for entry in &mut entries {
            entry.tags = self.get_tags_for_entry(&entry.id)?;
        }

        Ok(entries)
    }

    pub fn list_by_category(&self, category: &str) -> Result<Vec<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral
            FROM knowledge
            WHERE category_id = ?1
            ORDER BY title ASC
            "#,
        )?;

        let mut entries = stmt
            .query_map(params![category], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: None,
                    source_project: row.get(5)?,
                    source_agent: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type: row.get(11)?,
                    entry_type: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load tags for each entry
        for entry in &mut entries {
            entry.tags = self.get_tags_for_entry(&entry.id)?;
        }

        Ok(entries)
    }

    pub fn get(&self, id: &str) -> Result<Option<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral
            FROM knowledge
            WHERE id = ?1
            "#,
        )?;

        let mut entry = stmt
            .query_row(params![id], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: None,
                    source_project: row.get(5)?,
                    source_agent: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type: row.get(11)?,
                    entry_type: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                })
            })
            .ok();

        if let Some(ref mut e) = entry {
            e.tags = self.get_tags_for_entry(&e.id)?;
        }

        Ok(entry)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM knowledge WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    pub fn count(&self) -> Result<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn list_tables(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;

        let tables = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(tables)
    }

    pub fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO agents (id, description, domain, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                domain = excluded.domain,
                updated_at = excluded.updated_at
            "#,
            params![
                agent.id,
                agent.description,
                agent.domain,
                agent.created_at,
                agent.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, domain, created_at, updated_at FROM agents WHERE id = ?1",
        )?;

        let agent = stmt
            .query_row(params![id], |row| {
                Ok(Agent {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    domain: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .ok();

        Ok(agent)
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, domain, created_at, updated_at FROM agents ORDER BY id",
        )?;

        let agents = stmt
            .query_map([], |row| {
                Ok(Agent {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    domain: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(agents)
    }

    // Categories
    pub fn list_categories(&self) -> Result<Vec<Category>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, created_at FROM categories ORDER BY id")?;

        let categories = stmt
            .query_map([], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(categories)
    }

    pub fn get_category(&self, id: &str) -> Result<Option<Category>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, created_at FROM categories WHERE id = ?1")?;

        let category = stmt
            .query_row(params![id], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .ok();

        Ok(category)
    }

    // Projects
    pub fn list_projects(&self, active_only: bool) -> Result<Vec<Project>> {
        let query = if active_only {
            "SELECT id, name, path, repo_url, description, active, created_at, updated_at FROM projects WHERE active = 1 ORDER BY name"
        } else {
            "SELECT id, name, path, repo_url, description, active, created_at, updated_at FROM projects ORDER BY name"
        };

        let mut stmt = self.conn.prepare(query)?;

        let projects = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    repo_url: row.get(3)?,
                    description: row.get(4)?,
                    active: row.get::<_, i32>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(projects)
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, repo_url, description, active, created_at, updated_at FROM projects WHERE id = ?1"
        )?;

        let project = stmt
            .query_row(params![id], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    repo_url: row.get(3)?,
                    description: row.get(4)?,
                    active: row.get::<_, i32>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .ok();

        Ok(project)
    }

    pub fn upsert_project(&self, project: &Project) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO projects (id, name, path, repo_url, description, active, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                path = excluded.path,
                repo_url = excluded.repo_url,
                description = excluded.description,
                active = excluded.active,
                updated_at = excluded.updated_at
            "#,
            params![
                project.id,
                project.name,
                project.path,
                project.repo_url,
                project.description,
                project.active as i32,
                project.created_at,
                project.updated_at,
            ],
        )?;
        Ok(())
    }

    // Applicability Types
    pub fn list_applicability_types(&self) -> Result<Vec<ApplicabilityType>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, scope, created_at FROM applicability_types ORDER BY id",
        )?;

        let types = stmt
            .query_map([], |row| {
                Ok(ApplicabilityType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    scope: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(types)
    }

    pub fn upsert_applicability_type(&self, atype: &ApplicabilityType) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO applicability_types (id, description, scope, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                scope = excluded.scope
            "#,
            params![atype.id, atype.description, atype.scope, atype.created_at,],
        )?;
        Ok(())
    }

    // Source Types
    pub fn list_source_types(&self) -> Result<Vec<SourceType>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, created_at FROM source_types ORDER BY id")?;

        let types = stmt
            .query_map([], |row| {
                Ok(SourceType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(types)
    }

    // Entry Types
    pub fn list_entry_types(&self) -> Result<Vec<EntryType>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, created_at FROM entry_types ORDER BY id")?;

        let types = stmt
            .query_map([], |row| {
                Ok(EntryType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(types)
    }

    // Relationship Types
    pub fn list_relationship_types(&self) -> Result<Vec<RelationshipType>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, directional, created_at FROM relationship_types ORDER BY id",
        )?;

        let types = stmt
            .query_map([], |row| {
                Ok(RelationshipType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    directional: row.get::<_, i32>(2)? != 0,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(types)
    }

    // Session Types
    pub fn list_session_types(&self) -> Result<Vec<SessionType>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, created_at FROM session_types ORDER BY id")?;

        let types = stmt
            .query_map([], |row| {
                Ok(SessionType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(types)
    }

    // Sessions
    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO sessions (id, session_type_id, project_id, started_at, ended_at, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                session_type_id = excluded.session_type_id,
                project_id = excluded.project_id,
                ended_at = excluded.ended_at,
                metadata = excluded.metadata
            "#,
            params![
                session.id,
                session.session_type_id,
                session.project_id,
                session.started_at,
                session.ended_at,
                session.metadata,
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_type_id, project_id, started_at, ended_at, metadata FROM sessions WHERE id = ?1"
        )?;

        let session = stmt
            .query_row(params![id], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    session_type_id: row.get(1)?,
                    project_id: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })
            .ok();

        Ok(session)
    }

    pub fn list_sessions(&self, project_id: Option<&str>) -> Result<Vec<Session>> {
        let (query, params_vec): (&str, Vec<&str>) = match project_id {
            Some(pid) => (
                "SELECT id, session_type_id, project_id, started_at, ended_at, metadata FROM sessions WHERE project_id = ?1 ORDER BY started_at DESC",
                vec![pid],
            ),
            None => (
                "SELECT id, session_type_id, project_id, started_at, ended_at, metadata FROM sessions ORDER BY started_at DESC",
                vec![],
            ),
        };

        let mut stmt = self.conn.prepare(query)?;

        let sessions = stmt
            .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                Ok(Session {
                    id: row.get(0)?,
                    session_type_id: row.get(1)?,
                    project_id: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    metadata: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    // Junction table helpers - Tags
    pub fn get_tags_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM tags WHERE entry_id = ?1 ORDER BY tag")?;

        let tags = stmt
            .query_map(params![entry_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(tags)
    }

    pub fn set_tags_for_entry(&self, entry_id: &str, tags: &[String]) -> Result<()> {
        // Delete existing tags
        self.conn
            .execute("DELETE FROM tags WHERE entry_id = ?1", params![entry_id])?;

        // Insert new tags
        for tag in tags {
            self.conn.execute(
                "INSERT INTO tags (entry_id, tag) VALUES (?1, ?2)",
                params![entry_id, tag],
            )?;
        }

        Ok(())
    }

    // Junction table helpers - Applicability for Knowledge
    pub fn get_applicability_for_entry(&self, entry_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT applicability_id FROM knowledge_applicability WHERE entry_id = ?1 ORDER BY applicability_id",
        )?;

        let applicability = stmt
            .query_map(params![entry_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(applicability)
    }

    pub fn set_applicability_for_entry(&self, entry_id: &str, ids: &[String]) -> Result<()> {
        // Delete existing applicability
        self.conn.execute(
            "DELETE FROM knowledge_applicability WHERE entry_id = ?1",
            params![entry_id],
        )?;

        // Insert new applicability
        for id in ids {
            self.conn.execute(
                "INSERT INTO knowledge_applicability (entry_id, applicability_id) VALUES (?1, ?2)",
                params![entry_id, id],
            )?;
        }

        Ok(())
    }

    // Junction table helpers - Applicability for Projects
    pub fn get_applicability_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT applicability_id FROM project_applicability WHERE project_id = ?1 ORDER BY applicability_id",
        )?;

        let applicability = stmt
            .query_map(params![project_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(applicability)
    }

    pub fn set_applicability_for_project(&self, project_id: &str, ids: &[String]) -> Result<()> {
        // Delete existing applicability
        self.conn.execute(
            "DELETE FROM project_applicability WHERE project_id = ?1",
            params![project_id],
        )?;

        // Insert new applicability
        for id in ids {
            self.conn.execute(
                "INSERT INTO project_applicability (project_id, applicability_id) VALUES (?1, ?2)",
                params![project_id, id],
            )?;
        }

        Ok(())
    }

    // Junction table helpers - Tags for Projects
    pub fn get_tags_for_project(&self, project_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM project_tags WHERE project_id = ?1 ORDER BY tag")?;

        let tags = stmt
            .query_map(params![project_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(tags)
    }

    pub fn set_tags_for_project(&self, project_id: &str, tags: &[String]) -> Result<()> {
        // Delete existing tags
        self.conn.execute(
            "DELETE FROM project_tags WHERE project_id = ?1",
            params![project_id],
        )?;

        // Insert new tags
        for tag in tags {
            self.conn.execute(
                "INSERT INTO project_tags (project_id, tag) VALUES (?1, ?2)",
                params![project_id, tag],
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, category: &str, title: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            category: category.to_string(),
            title: title.to_string(),
            body: None,
            summary: None,
            applicability: None,
            source_project: None,
            source_agent: None,
            file_path: None,
            tags: vec![],
            created_at: None,
            updated_at: None,
            content_hash: None,
            source_type: None,
            entry_type: None,
            session_id: None,
            ephemeral: false,
        }
    }

    #[test]
    fn test_crud_operations() {
        let db = Database::open_in_memory().unwrap();

        // Insert
        let entry = make_entry("kn-test1", "pattern", "Test Pattern");
        db.upsert_knowledge(&entry).unwrap();
        assert_eq!(db.count().unwrap(), 1);

        // Get
        let fetched = db.get("kn-test1").unwrap().unwrap();
        assert_eq!(fetched.title, "Test Pattern");

        // Update (upsert)
        let updated = make_entry("kn-test1", "pattern", "Updated Pattern");
        db.upsert_knowledge(&updated).unwrap();
        assert_eq!(db.count().unwrap(), 1);
        let fetched = db.get("kn-test1").unwrap().unwrap();
        assert_eq!(fetched.title, "Updated Pattern");

        // Delete
        assert!(db.delete("kn-test1").unwrap());
        assert_eq!(db.count().unwrap(), 0);
        assert!(db.get("kn-test1").unwrap().is_none());

        // Delete non-existent
        assert!(!db.delete("kn-nonexistent").unwrap());
    }

    #[test]
    fn test_search() {
        let db = Database::open_in_memory().unwrap();

        db.upsert_knowledge(&make_entry("kn-1", "pattern", "Unicode Parsing"))
            .unwrap();
        db.upsert_knowledge(&make_entry("kn-2", "technique", "Error Handling"))
            .unwrap();
        db.upsert_knowledge(&make_entry("kn-3", "pattern", "Unicode Encoding"))
            .unwrap();

        let results = db.search("unicode").unwrap();
        assert_eq!(results.len(), 2);

        let results = db.search("error").unwrap();
        assert_eq!(results.len(), 1);

        let results = db.search("nonexistent").unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_list_by_category() {
        let db = Database::open_in_memory().unwrap();

        db.upsert_knowledge(&make_entry("kn-1", "pattern", "Pattern 1"))
            .unwrap();
        db.upsert_knowledge(&make_entry("kn-2", "pattern", "Pattern 2"))
            .unwrap();
        db.upsert_knowledge(&make_entry("kn-3", "technique", "Technique 1"))
            .unwrap();

        let patterns = db.list_by_category("pattern").unwrap();
        assert_eq!(patterns.len(), 2);

        let techniques = db.list_by_category("technique").unwrap();
        assert_eq!(techniques.len(), 1);

        let insights = db.list_by_category("insight").unwrap();
        assert_eq!(insights.len(), 0);
    }
}
