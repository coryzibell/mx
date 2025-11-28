use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::knowledge::KnowledgeEntry;

const SCHEMA_VERSION: i32 = 2;

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub description: Option<String>,
    pub domain: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
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
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM pragma_user_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < SCHEMA_VERSION {
            self.conn.execute_batch(include_str!("schema.sql"))?;
            self.conn
                .execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION), [])?;
        }

        Ok(())
    }

    pub fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO knowledge (id, category, title, body, summary, applicability,
                                   source_project, source_agent, file_path, tags,
                                   created_at, updated_at, content_hash,
                                   source_type, entry_type, session_id, ephemeral)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                category = excluded.category,
                title = excluded.title,
                body = excluded.body,
                summary = excluded.summary,
                applicability = excluded.applicability,
                source_project = excluded.source_project,
                source_agent = excluded.source_agent,
                file_path = excluded.file_path,
                tags = excluded.tags,
                updated_at = excluded.updated_at,
                content_hash = excluded.content_hash,
                source_type = excluded.source_type,
                entry_type = excluded.entry_type,
                session_id = excluded.session_id,
                ephemeral = excluded.ephemeral
            "#,
            params![
                entry.id,
                entry.category,
                entry.title,
                entry.body,
                entry.summary,
                entry.applicability,
                entry.source_project,
                entry.source_agent,
                entry.file_path,
                serde_json::to_string(&entry.tags)?,
                entry.created_at,
                entry.updated_at,
                entry.content_hash,
                entry.source_type,
                entry.entry_type,
                entry.session_id,
                entry.ephemeral,
            ],
        )?;

        // Update tags table
        self.conn
            .execute("DELETE FROM tags WHERE entry_id = ?1", params![entry.id])?;
        for tag in &entry.tags {
            self.conn.execute(
                "INSERT INTO tags (entry_id, tag) VALUES (?1, ?2)",
                params![entry.id, tag],
            )?;
        }

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<KnowledgeEntry>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category, title, body, summary, applicability,
                   source_project, source_agent, file_path, tags,
                   created_at, updated_at, content_hash,
                   source_type, entry_type, session_id, ephemeral
            FROM knowledge
            WHERE title LIKE ?1 OR body LIKE ?1 OR summary LIKE ?1
            ORDER BY updated_at DESC
            "#,
        )?;

        let entries = stmt
            .query_map(params![pattern], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: row.get(5)?,
                    source_project: row.get(6)?,
                    source_agent: row.get(7)?,
                    file_path: row.get(8)?,
                    tags: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    content_hash: row.get(12)?,
                    source_type: row.get(13)?,
                    entry_type: row.get(14)?,
                    session_id: row.get(15)?,
                    ephemeral: row.get::<_, i32>(16)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    pub fn list_by_category(&self, category: &str) -> Result<Vec<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category, title, body, summary, applicability,
                   source_project, source_agent, file_path, tags,
                   created_at, updated_at, content_hash,
                   source_type, entry_type, session_id, ephemeral
            FROM knowledge
            WHERE category = ?1
            ORDER BY title ASC
            "#,
        )?;

        let entries = stmt
            .query_map(params![category], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: row.get(5)?,
                    source_project: row.get(6)?,
                    source_agent: row.get(7)?,
                    file_path: row.get(8)?,
                    tags: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    content_hash: row.get(12)?,
                    source_type: row.get(13)?,
                    entry_type: row.get(14)?,
                    session_id: row.get(15)?,
                    ephemeral: row.get::<_, i32>(16)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    pub fn get(&self, id: &str) -> Result<Option<KnowledgeEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, category, title, body, summary, applicability,
                   source_project, source_agent, file_path, tags,
                   created_at, updated_at, content_hash,
                   source_type, entry_type, session_id, ephemeral
            FROM knowledge
            WHERE id = ?1
            "#,
        )?;

        let entry = stmt
            .query_row(params![id], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    summary: row.get(4)?,
                    applicability: row.get(5)?,
                    source_project: row.get(6)?,
                    source_agent: row.get(7)?,
                    file_path: row.get(8)?,
                    tags: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                    content_hash: row.get(12)?,
                    source_type: row.get(13)?,
                    entry_type: row.get(14)?,
                    session_id: row.get(15)?,
                    ephemeral: row.get::<_, i32>(16)? != 0,
                })
            })
            .ok();

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
