use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::knowledge::KnowledgeEntry;

const SCHEMA_VERSION: i32 = 1;

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

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        // Check schema version
        let version: i32 = self.conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM pragma_user_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < SCHEMA_VERSION {
            self.conn.execute_batch(include_str!("schema.sql"))?;
            self.conn.execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION), [])?;
        }

        Ok(())
    }

    pub fn upsert_knowledge(&self, entry: &KnowledgeEntry) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO knowledge (id, category, title, body, summary, applicability,
                                   source_project, source_agent, file_path, tags,
                                   created_at, updated_at, content_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
                content_hash = excluded.content_hash
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
            ],
        )?;

        // Update tags table
        self.conn.execute("DELETE FROM tags WHERE entry_id = ?1", params![entry.id])?;
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
                   created_at, updated_at, content_hash
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
                   created_at, updated_at, content_hash
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
                   created_at, updated_at, content_hash
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
                })
            })
            .ok();

        Ok(entry)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let rows = self.conn.execute("DELETE FROM knowledge WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    pub fn count(&self) -> Result<usize> {
        let count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM knowledge",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}
