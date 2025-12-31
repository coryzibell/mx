use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::Path;

use crate::knowledge::KnowledgeEntry;
use crate::store::KnowledgeStore;
use serde::{Deserialize, Serialize};

// Schema version - kept for future migrations
#[allow(dead_code)]
const SCHEMA_VERSION: i32 = 5;

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

// Type definitions - used by database queries
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceType {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryType {
    pub id: String,
    pub description: String,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentType {
    pub id: String,
    pub description: String,
    pub file_extensions: Option<String>,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipType {
    pub id: String,
    pub description: String,
    pub directional: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub from_entry_id: String,
    pub to_entry_id: String,
    pub relationship_type: String,
    pub created_at: String,
}

#[allow(dead_code)]
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
                // Fresh install - apply full v5 schema
                self.conn.execute_batch(include_str!("schema.sql"))?;
                // Seed content types
                self.conn.execute_batch(r#"
                    INSERT OR IGNORE INTO content_types (id, description, file_extensions, created_at) VALUES
                        ('text', 'Plain text or markdown documents', 'md,txt,text', datetime('now')),
                        ('code', 'Source code or scripts', 'py,rs,js,ts,sh,bash,rb,go,java,c,cpp,h', datetime('now')),
                        ('config', 'Configuration files', 'json,yaml,yml,toml,xml,ini,env', datetime('now')),
                        ('data', 'Data files or test fixtures', 'json,csv,sql,fiche,schema', datetime('now')),
                        ('binary', 'Binary or encoded content', 'bin,dat,b64', datetime('now'));
                "#)?;
                self.conn.execute("PRAGMA user_version = 5", [])?;
            }
            2 => {
                // Migrate from v2 to v3
                eprintln!("Migrating Zion schema from v2 to v3...");
                self.conn
                    .execute_batch(include_str!("migrations/v2_to_v3.sql"))?;
                eprintln!("Migration complete.");
                // Fall through to v3->v4 migration
                self.conn
                    .execute_batch(include_str!("migrations/v3_to_v4.sql"))?;
                eprintln!("Migrated to v4.");
            }
            3 => {
                // Migrate from v3 to v4
                eprintln!("Migrating Zion schema from v3 to v4...");
                self.conn
                    .execute_batch(include_str!("migrations/v3_to_v4.sql"))?;
                eprintln!("Migration complete.");
                // Fall through to v4->v5 migration
                eprintln!("Migrating Zion schema from v4 to v5...");
                self.conn
                    .execute_batch(include_str!("migrations/v4_to_v5.sql"))?;
                eprintln!("Migrated to v5.");
            }
            4 => {
                // Migrate from v4 to v5
                eprintln!("Migrating Zion schema from v4 to v5...");
                self.conn
                    .execute_batch(include_str!("migrations/v4_to_v5.sql"))?;
                eprintln!("Migration complete.");
            }
            5 => {
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
                                   source_type_id, entry_type_id, session_id, ephemeral, content_type_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
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
                ephemeral = excluded.ephemeral,
                content_type_id = excluded.content_type_id
            "#,
            params![
                entry.id,
                entry.category_id,
                entry.title,
                entry.body,
                entry.summary,
                entry.source_project_id,
                entry.source_agent_id,
                entry.file_path,
                entry.created_at,
                entry.updated_at,
                entry.content_hash,
                entry.source_type_id,
                entry.entry_type_id,
                entry.session_id,
                entry.ephemeral,
                entry.content_type_id,
            ],
        )?;

        // Update tags junction table
        self.set_tags_for_entry(&entry.id, &entry.tags)?;

        // Update applicability junction table
        self.set_applicability_for_entry(&entry.id, &entry.applicability)?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<KnowledgeEntry>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral, content_type_id
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
                    category_id: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: vec![],
                    source_project_id: row.get(5)?,
                    source_agent_id: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type_id: row.get(11)?,
                    entry_type_id: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                    content_type_id: row.get(15)?,
                    owner: None,
                    visibility: "public".to_string(),
                    resonance: 0,
                    resonance_type: None,
                    last_activated: None,
                    activation_count: 0,
                    decay_rate: 0.0,
                    anchors: vec![],
                    wake_phrases: vec![],
                    wake_order: None,
                    wake_phrase: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load tags and applicability for each entry
        for entry in &mut entries {
            entry.tags = self.get_tags_for_entry(&entry.id)?;
            entry.applicability = self.get_applicability_for_entry(&entry.id)?;
        }

        Ok(entries)
    }

    pub fn list_by_category(&self, category: &str) -> Result<Vec<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral, content_type_id
            FROM knowledge
            WHERE category_id = ?1
            ORDER BY title ASC
            "#,
        )?;

        let mut entries = stmt
            .query_map(params![category], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category_id: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: vec![],
                    source_project_id: row.get(5)?,
                    source_agent_id: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type_id: row.get(11)?,
                    entry_type_id: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                    content_type_id: row.get(15)?,
                    owner: None,
                    visibility: "public".to_string(),
                    resonance: 0,
                    resonance_type: None,
                    last_activated: None,
                    activation_count: 0,
                    decay_rate: 0.0,
                    anchors: vec![],
                    wake_phrases: vec![],
                    wake_order: None,
                    wake_phrase: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Load tags and applicability for each entry
        for entry in &mut entries {
            entry.tags = self.get_tags_for_entry(&entry.id)?;
            entry.applicability = self.get_applicability_for_entry(&entry.id)?;
        }

        Ok(entries)
    }

    pub fn get(&self, id: &str) -> Result<Option<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category_id, title, body, summary,
                   source_project_id, source_agent_id, file_path,
                   created_at, updated_at, content_hash,
                   source_type_id, entry_type_id, session_id, ephemeral, content_type_id
            FROM knowledge
            WHERE id = ?1
            "#,
        )?;

        let mut entry = stmt
            .query_row(params![id], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category_id: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: vec![],
                    source_project_id: row.get(5)?,
                    source_agent_id: row.get(6)?,
                    file_path: row.get(7)?,
                    tags: vec![],
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    content_hash: row.get(10)?,
                    source_type_id: row.get(11)?,
                    entry_type_id: row.get(12)?,
                    session_id: row.get(13)?,
                    ephemeral: row.get::<_, i32>(14)? != 0,
                    content_type_id: row.get(15)?,
                    owner: None,
                    visibility: "public".to_string(),
                    resonance: 0,
                    resonance_type: None,
                    last_activated: None,
                    activation_count: 0,
                    decay_rate: 0.0,
                    anchors: vec![],
                    wake_phrases: vec![],
                    wake_order: None,
                    wake_phrase: None,
                })
            })
            .ok();

        if let Some(ref mut e) = entry {
            e.tags = self.get_tags_for_entry(&e.id)?;
            e.applicability = self.get_applicability_for_entry(&e.id)?;
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

    pub fn upsert_category(&self, category: &Category) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO categories (id, description, created_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(id) DO UPDATE SET
                description = excluded.description
            "#,
            params![category.id, category.description, category.created_at],
        )?;
        Ok(())
    }

    pub fn delete_category(&self, id: &str) -> Result<bool> {
        // Check if any knowledge entries use this category
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM knowledge WHERE category_id = ?1")?;
        let count: i64 = stmt.query_row(params![id], |row| row.get(0))?;

        if count > 0 {
            anyhow::bail!(
                "Cannot remove category '{}': {} entries still use it",
                id,
                count
            );
        }

        // Delete the category
        let rows = self
            .conn
            .execute("DELETE FROM categories WHERE id = ?1", params![id])?;
        Ok(rows > 0)
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

    // Content Types
    pub fn list_content_types(&self) -> Result<Vec<ContentType>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, file_extensions, created_at FROM content_types ORDER BY id",
        )?;

        let types = stmt
            .query_map([], |row| {
                Ok(ContentType {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    file_extensions: row.get(2)?,
                    created_at: row.get(3)?,
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

    // Relationships
    pub fn list_relationships_for_entry(&self, entry_id: &str) -> Result<Vec<Relationship>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, from_entry_id, to_entry_id, relationship_type, created_at
            FROM relationships
            WHERE from_entry_id = ?1 OR to_entry_id = ?1
            ORDER BY created_at DESC
            "#,
        )?;

        let relationships = stmt
            .query_map(params![entry_id], |row| {
                Ok(Relationship {
                    id: row.get(0)?,
                    from_entry_id: row.get(1)?,
                    to_entry_id: row.get(2)?,
                    relationship_type: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(relationships)
    }

    pub fn add_relationship(&self, from_id: &str, to_id: &str, rel_type: &str) -> Result<String> {
        // Validate relationship type exists
        let valid_types = self.list_relationship_types()?;
        if !valid_types.iter().any(|t| t.id == rel_type) {
            anyhow::bail!("Invalid relationship type: {}", rel_type);
        }

        // Validate both entries exist
        if self.get(from_id)?.is_none() {
            anyhow::bail!("Source entry not found: {}", from_id);
        }
        if self.get(to_id)?.is_none() {
            anyhow::bail!("Target entry not found: {}", to_id);
        }

        // Generate ID
        let id = format!(
            "rel-{}-{}-{}",
            &from_id[3..8],
            &to_id[3..8],
            &rel_type[..3.min(rel_type.len())]
        );
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            r#"
            INSERT INTO relationships (id, from_entry_id, to_entry_id, relationship_type, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![id, from_id, to_id, rel_type, now],
        )?;

        Ok(id)
    }

    pub fn delete_relationship(&self, id: &str) -> Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM relationships WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}

// ============================================================================
// KNOWLEDGESTORE TRAIT IMPLEMENTATION
// ============================================================================

impl KnowledgeStore for Database {
    fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.upsert_knowledge(entry)
    }

    fn get(&self, id: &str, _ctx: &crate::store::AgentContext) -> Result<Option<KnowledgeEntry>> {
        // SQLite backend doesn't support privacy filtering yet - always return entry if it exists
        self.get(id)
    }

    fn delete(&self, id: &str) -> Result<bool> {
        self.delete(id)
    }

    fn search(
        &self,
        query: &str,
        _ctx: &crate::store::AgentContext,
        _filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        // SQLite backend doesn't support privacy filtering or resonance filtering yet - return all results
        self.search(query)
    }

    fn list_by_category(
        &self,
        category: &str,
        _ctx: &crate::store::AgentContext,
        _filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        // SQLite backend doesn't support privacy filtering or resonance filtering yet - return all results
        self.list_by_category(category)
    }

    fn count(&self) -> Result<usize> {
        self.count()
    }

    fn wake_cascade(
        &self,
        _ctx: &crate::store::AgentContext,
        _limit: usize,
        _min_resonance: Option<i32>,
        _days: i64,
    ) -> Result<crate::store::WakeCascade> {
        // SQLite backend doesn't support wake cascade yet - return empty cascade
        Ok(crate::store::WakeCascade {
            core: vec![],
            recent: vec![],
            bridges: vec![],
        })
    }

    fn update_activations(&self, _ids: &[String]) -> Result<()> {
        // SQLite backend doesn't support activation tracking yet - no-op
        Ok(())
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

    fn list_projects(&self, active_only: bool) -> Result<Vec<Project>> {
        self.list_projects(active_only)
    }

    fn get_project(&self, id: &str) -> Result<Option<Project>> {
        self.get_project(id)
    }

    fn upsert_project(&self, project: &Project) -> Result<()> {
        self.upsert_project(project)
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
        self.list_relationships_for_entry(entry_id)
    }

    fn add_relationship(&self, from: &str, to: &str, rel_type: &str) -> Result<String> {
        self.add_relationship(from, to, rel_type)
    }

    fn delete_relationship(&self, id: &str) -> Result<bool> {
        self.delete_relationship(id)
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

    fn seed_test_db(db: &Database) {
        let now = chrono::Utc::now().to_rfc3339();

        // Insert required categories
        db.conn.execute(
            "INSERT OR IGNORE INTO categories (id, description, created_at) VALUES (?1, ?2, ?3)",
            params!["pattern", "Pattern", &now],
        ).unwrap();
        db.conn.execute(
            "INSERT OR IGNORE INTO categories (id, description, created_at) VALUES (?1, ?2, ?3)",
            params!["technique", "Technique", &now],
        ).unwrap();

        // Insert required source types
        db.conn.execute(
            "INSERT OR IGNORE INTO source_types (id, description, created_at) VALUES (?1, ?2, ?3)",
            params!["manual", "Manual entry", &now],
        ).unwrap();

        // Insert required entry types
        db.conn.execute(
            "INSERT OR IGNORE INTO entry_types (id, description, created_at) VALUES (?1, ?2, ?3)",
            params!["primary", "Primary entry", &now],
        ).unwrap();
    }

    fn make_entry(id: &str, category: &str, title: &str) -> KnowledgeEntry {
        let now = chrono::Utc::now().to_rfc3339();
        KnowledgeEntry {
            id: id.to_string(),
            category_id: category.to_string(),
            title: title.to_string(),
            body: None,
            summary: None,
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            tags: vec![],
            created_at: Some(now.clone()),
            updated_at: Some(now),
            content_hash: Some("test-hash".to_string()),
            source_type_id: Some("manual".to_string()),
            entry_type_id: Some("primary".to_string()),
            session_id: None,
            ephemeral: false,
            content_type_id: Some("text".to_string()),
            owner: None,
            visibility: "public".to_string(),
            resonance: 0,
            resonance_type: None,
            last_activated: None,
            activation_count: 0,
            decay_rate: 0.0,
            anchors: vec![],
            wake_phrase: None,
        }
    }

    #[test]
    fn test_crud_operations() {
        let db = Database::open_in_memory().unwrap();
        seed_test_db(&db);

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
        seed_test_db(&db);

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
        seed_test_db(&db);

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
