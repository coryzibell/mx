use anyhow::{Context, Result};
use std::path::Path;
use std::sync::OnceLock;
use surrealdb::Surreal;
use surrealdb::engine::local::RocksDb;
use tokio::runtime::Runtime;

/// Embedded SurrealDB schema - applied on database open
const SCHEMA: &str = include_str!("../schema/surrealdb-schema.surql");

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

        // Connect to RocksDB backend
        let db = Surreal::new::<RocksDb>(path)
            .await
            .with_context(|| format!("Failed to open SurrealDB at {:?}", path))?;

        // Use namespace and database
        db.use_ns("zion")
            .use_db("knowledge")
            .await
            .context("Failed to set namespace and database")?;

        // Apply schema (idempotent)
        db.query(SCHEMA)
            .await
            .context("Failed to apply database schema")?;

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
}
