use serde::{Deserialize, Serialize};
use surrealdb::RecordId as SurrealRecordId;
use surrealdb::sql::Thing;

// The with_db! macro must be defined BEFORE mod declarations so that
// submodules can use it (macro_rules! macros are visible to child modules
// when defined before the `mod` statement).

/// Macro to execute code with the appropriate database connection (embedded or network)
///
/// This macro handles the connection type dispatch, allowing the same query code
/// to work with both embedded (SurrealKV) and network (WebSocket) connections.
///
/// # Usage
/// ```rust,ignore
/// with_db!(self, db, {
///     db.query(&sql).bind(("key", value)).await?
/// })
/// ```
macro_rules! with_db {
    ($self:expr, $db:ident, $body:expr) => {
        match &$self.conn {
            SurrealConnection::Embedded($db) => $body,
            SurrealConnection::Network($db) => $body,
        }
    };
}

mod connection;
mod knowledge;
mod lookups;
mod queries;
mod relationships;
mod trait_impl;

// Re-export connection types that external code needs
pub use connection::SurrealConnection;

// normalize_datetime is used by submodules via `super::connection::normalize_datetime`

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

/// SurrealDB-backed knowledge store
pub struct SurrealDatabase {
    conn: SurrealConnection,
}

/// Helper for SELECT-based existence checks on `relates_to` edges.
/// Used by both `delete_relationship_by_id_async` and `delete_relationship_async`.
#[derive(Debug, Deserialize)]
struct ExistsRow {
    id: String,
}

#[cfg(test)]
mod tests;
