use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Pre-mutation content backup (Issue #206)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBackup {
    pub id: String,
    pub entry_id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub content_hash: String,
    pub operation: String,
    #[serde(default)]
    pub source_agent: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
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
