use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;
use surrealdb::RecordId as SurrealRecordId;
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb::engine::remote::ws::{Client as WsClient, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::sql::{Thing, Value};
use tokio::runtime::Runtime;

use crate::db::{
    Agent, ApplicabilityType, Category, ContentType, EntryType, Project, Relationship,
    RelationshipType, Session, SessionType, SourceType,
};
use crate::knowledge::KnowledgeEntry;
use crate::store::KnowledgeStore;

// ============================================================================
// CONNECTION MODE CONFIGURATION
// ============================================================================

/// Connection mode for SurrealDB
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SurrealMode {
    /// Embedded SurrealKV (local file-based, default)
    #[default]
    Embedded,
    /// Network connection via WebSocket
    Network,
}

/// Configuration for SurrealDB connection
///
/// Parsed from environment variables:
/// - `MX_SURREAL_MODE`: "embedded" (default) or "network"
/// - `MX_SURREAL_URL`: WebSocket URL for network mode (default: ws://localhost:8000)
/// - `MX_SURREAL_USER`: Username for network auth (default: root)
/// - `MX_SURREAL_PASS`: Password for network auth (direct value)
/// - `MX_SURREAL_PASS_FILE`: Path to file containing password (e.g., agenix secret)
/// - `MX_SURREAL_NS`: Namespace (default: memory)
/// - `MX_SURREAL_DB`: Database name (default: knowledge)
#[derive(Debug, Clone)]
pub struct SurrealConfig {
    /// Connection mode
    pub mode: SurrealMode,
    /// WebSocket URL for network mode
    pub url: String,
    /// Username for network authentication
    pub user: String,
    /// Password for network authentication
    pub pass: Option<String>,
    /// SurrealDB namespace
    pub namespace: String,
    /// SurrealDB database name
    pub database: String,
}

impl Default for SurrealConfig {
    fn default() -> Self {
        Self {
            mode: SurrealMode::Embedded,
            url: "ws://localhost:8000".to_string(),
            user: "root".to_string(),
            pass: None,
            namespace: "memory".to_string(),
            database: "knowledge".to_string(),
        }
    }
}

impl SurrealConfig {
    /// Parse configuration from environment variables
    pub fn from_env() -> Self {
        let mode = match std::env::var("MX_SURREAL_MODE")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "network" => SurrealMode::Network,
            _ => SurrealMode::Embedded,
        };

        let url =
            std::env::var("MX_SURREAL_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string());

        let user = std::env::var("MX_SURREAL_USER").unwrap_or_else(|_| "root".to_string());

        // Get password: try direct value first, then file path, filter empty strings
        let pass = std::env::var("MX_SURREAL_PASS")
            .ok()
            .or_else(|| {
                // Try reading from file path (e.g., agenix secret)
                std::env::var("MX_SURREAL_PASS_FILE")
                    .ok()
                    .and_then(|path| std::fs::read_to_string(path).ok())
            })
            .map(|s| s.trim().to_string())
            .filter(|p| !p.is_empty());

        let namespace = std::env::var("MX_SURREAL_NS").unwrap_or_else(|_| "memory".to_string());

        let database = std::env::var("MX_SURREAL_DB").unwrap_or_else(|_| "knowledge".to_string());

        Self {
            mode,
            url,
            user,
            pass,
            namespace,
            database,
        }
    }

    /// Check if we're in network mode
    pub fn is_network(&self) -> bool {
        self.mode == SurrealMode::Network
    }
}

/// Embedded SurrealDB schema - applied on database open
const SCHEMA: &str = include_str!("../schema/surrealdb-schema.surql");

/// Tag record for SurrealDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// DTO for deserializing knowledge records from SurrealDB queries.
///
/// SurrealDB returns record links as `Thing` types, which don't deserialize
/// to serde_json::Value properly. This DTO expects queries to use:
///   - `meta::id(id) AS id` for the record ID
///   - `meta::id(category) AS category_id` for record links
///   - `<string>created_at AS created_at` for datetime conversion
///
/// This allows direct deserialization without manual JSON field extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurrealKnowledgeRecord {
    /// Record ID (from `meta::id(id)`)
    pub id: String,

    /// Entry title
    pub title: String,

    /// Full body content
    #[serde(default)]
    pub body: Option<String>,

    /// Brief summary
    #[serde(default)]
    pub summary: Option<String>,

    /// Source file path (for markdown-sourced entries)
    #[serde(default)]
    pub file_path: Option<String>,

    /// Content hash for change detection
    #[serde(default)]
    pub content_hash: Option<String>,

    /// Whether this is ephemeral/session-scoped
    #[serde(default)]
    pub ephemeral: bool,

    /// Owner ID for private entries
    #[serde(default)]
    pub owner: Option<String>,

    /// Visibility: "public" or "private"
    #[serde(default = "default_visibility")]
    pub visibility: String,

    // === Record links (converted to strings via meta::id()) ===
    /// Category ID (from `meta::id(category)`)
    pub category_id: String,

    /// Source type ID (from `meta::id(source_type)`)
    #[serde(default)]
    pub source_type_id: Option<String>,

    /// Entry type ID (from `meta::id(entry_type)`)
    #[serde(default)]
    pub entry_type_id: Option<String>,

    /// Content type ID (from `meta::id(content_type)`)
    #[serde(default)]
    pub content_type_id: Option<String>,

    /// Source project ID
    #[serde(default)]
    pub source_project_id: Option<String>,

    /// Source agent ID
    #[serde(default)]
    pub source_agent_id: Option<String>,

    /// Session ID
    #[serde(default)]
    pub session_id: Option<String>,

    // === Timestamps (converted to strings via <string>cast) ===
    /// Created timestamp (from `<string>created_at`)
    #[serde(default)]
    pub created_at: Option<String>,

    /// Updated timestamp (from `<string>updated_at`)
    #[serde(default)]
    pub updated_at: Option<String>,

    // === Resonance fields (for wake-up cascade) ===
    /// Resonance level (1-10, with overflow for transcendent)
    #[serde(default)]
    pub resonance: i32,

    /// Resonance type: foundational, transformative, relational, operational, ephemeral
    #[serde(default)]
    pub resonance_type: Option<String>,

    /// Last activated timestamp
    #[serde(default)]
    pub last_activated: Option<String>,

    /// Number of times activated
    #[serde(default)]
    pub activation_count: i32,

    /// Decay rate (0.0-1.0)
    #[serde(default)]
    pub decay_rate: f64,

    /// Anchor IDs (related blooms this connects to)
    #[serde(default)]
    pub anchors: Vec<String>,

    // Issue #72: Multiple wake phrases
    #[serde(default)]
    pub wake_phrases: Vec<String>,

    // Issue #73: Custom wake order
    #[serde(default)]
    pub wake_order: Option<i32>,

    /// DEPRECATED: Wake phrase for memory ritual verification (kept for backward compat)
    #[serde(default)]
    pub wake_phrase: Option<String>,

    // === Vector embeddings (PR #89) ===
    /// 768-dimensional embedding vector (BGE-Base-EN-v1.5)
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,

    /// Model ID that generated the embedding
    #[serde(default)]
    pub embedding_model: Option<String>,

    /// Timestamp when embedded
    #[serde(default)]
    pub embedded_at: Option<String>,
}

fn default_visibility() -> String {
    "public".to_string()
}

impl SurrealKnowledgeRecord {
    /// Convert to domain KnowledgeEntry, fetching tags and applicability
    pub fn into_knowledge_entry(
        self,
        tags: Vec<String>,
        applicability: Vec<String>,
    ) -> KnowledgeEntry {
        KnowledgeEntry {
            id: format!("kn-{}", self.id),
            category_id: self.category_id,
            title: self.title,
            body: self.body,
            summary: self.summary,
            file_path: self.file_path,
            content_hash: self.content_hash,
            ephemeral: self.ephemeral,
            owner: self.owner,
            visibility: self.visibility,
            source_type_id: self.source_type_id,
            entry_type_id: self.entry_type_id,
            content_type_id: self.content_type_id,
            source_project_id: self.source_project_id,
            source_agent_id: self.source_agent_id,
            session_id: self.session_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tags,
            applicability,
            resonance: self.resonance,
            resonance_type: self.resonance_type,
            last_activated: self.last_activated,
            activation_count: self.activation_count,
            decay_rate: self.decay_rate,
            anchors: self.anchors,
            wake_phrases: self.wake_phrases,
            wake_order: self.wake_order,
            wake_phrase: self.wake_phrase,
            embedding: self.embedding,
            embedding_model: self.embedding_model,
            embedded_at: self.embedded_at,
        }
    }
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

/// Connection abstraction for SurrealDB - supports both embedded and network modes
pub enum SurrealConnection {
    /// Embedded SurrealKV database (local file-based)
    Embedded(Surreal<surrealdb::engine::local::Db>),
    /// Network connection via WebSocket
    Network(Surreal<WsClient>),
}

/// SurrealDB-backed knowledge store
pub struct SurrealDatabase {
    conn: SurrealConnection,
}

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

impl SurrealDatabase {}

impl SurrealDatabase {
    /// Get or initialize the global tokio runtime
    fn runtime() -> &'static Runtime {
        static RT: OnceLock<Runtime> = OnceLock::new();
        RT.get_or_init(|| Runtime::new().expect("Failed to create tokio runtime"))
    }

    /// Open database at path, create if not exists, apply schema
    ///
    /// This method checks environment variables first - if `MX_SURREAL_MODE=network`,
    /// the path is ignored and a network connection is established instead.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = SurrealConfig::from_env();
        Self::runtime().block_on(Self::open_with_config_async(path, &config, false))
    }

    /// Open database with verbose control
    pub fn open_with_verbose<P: AsRef<Path>>(path: P, verbose: bool) -> Result<Self> {
        let config = SurrealConfig::from_env();
        Self::runtime().block_on(Self::open_with_config_async(path, &config, verbose))
    }

    /// Connect using explicit configuration
    ///
    /// For embedded mode, `path` specifies the database location.
    /// For network mode, `path` is ignored.
    pub fn connect<P: AsRef<Path>>(path: P, config: &SurrealConfig) -> Result<Self> {
        Self::runtime().block_on(Self::open_with_config_async(path, config, false))
    }

    /// Internal: open with config, branching on mode
    async fn open_with_config_async<P: AsRef<Path>>(
        path: P,
        config: &SurrealConfig,
        verbose: bool,
    ) -> Result<Self> {
        match config.mode {
            SurrealMode::Embedded => Self::open_embedded_async(path, config, verbose).await,
            SurrealMode::Network => Self::open_network_async(config, verbose).await,
        }
    }

    /// Open embedded SurrealKV database
    async fn open_embedded_async<P: AsRef<Path>>(
        path: P,
        config: &SurrealConfig,
        verbose: bool,
    ) -> Result<Self> {
        let path = path.as_ref();

        // Diagnostic: Log connection mode (only if verbose)
        if verbose {
            eprintln!(
                "[mx] Connecting to SurrealDB in embedded mode: {}",
                path.display()
            );
        }

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create database directory: {:?}", parent))?;
        }

        // Connect to SurrealKv backend
        let db = Surreal::new::<SurrealKv>(path).await.with_context(|| {
            format!(
                "Failed to open SurrealDB at {} (check file permissions and disk space)",
                path.display()
            )
        })?;

        // Use namespace and database from config
        if verbose {
            eprintln!(
                "[mx] Using namespace '{}' and database '{}'",
                config.namespace, config.database
            );
        }
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .context("Failed to set namespace and database")?;

        // Apply schema (idempotent)
        if verbose {
            eprintln!("[mx] Applying database schema");
        }
        let mut response = db
            .query(SCHEMA)
            .await
            .context("Failed to apply database schema")?;

        // Check for errors - schema application returns multiple results
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Schema application failed: {:?}", errors));
        }

        if verbose {
            eprintln!("[mx] Embedded connection established successfully");
        }

        Ok(Self {
            conn: SurrealConnection::Embedded(db),
        })
    }

    /// Check if URL is localhost (safe for unencrypted traffic)
    fn is_localhost_url(url: &str) -> bool {
        url.contains("://localhost") || url.contains("://127.0.0.1") || url.contains("://[::1]")
    }

    /// Strip protocol prefix from WebSocket URL
    ///
    /// The surrealdb crate expects just `host:port`, not `ws://host:port`.
    /// Users may provide the full URL with protocol, so we strip it if present.
    fn sanitize_ws_url(url: &str) -> String {
        url.strip_prefix("ws://")
            .or_else(|| url.strip_prefix("wss://"))
            .unwrap_or(url)
            .to_string()
    }

    /// Open network connection via WebSocket
    ///
    /// Authenticates with the remote SurrealDB server using credentials from config.
    async fn open_network_async(config: &SurrealConfig, verbose: bool) -> Result<Self> {
        // Diagnostic: Log connection attempt (to stderr, doesn't interfere with stdout)
        if verbose {
            eprintln!(
                "[mx] Connecting to SurrealDB in network mode: {}",
                config.url
            );
        }

        // Security warning: credentials over unencrypted WebSocket to non-localhost
        // (Always show warnings, regardless of verbose flag)
        if config.pass.is_some()
            && config.url.starts_with("ws://")
            && !Self::is_localhost_url(&config.url)
        {
            eprintln!(
                "[mx] WARNING: Sending credentials over unencrypted WebSocket to {}",
                config.url
            );
            eprintln!("[mx] WARNING: Consider using wss:// (TLS) for secure authentication");
        }

        // Strip protocol prefix from URL - surrealdb crate expects just host:port
        let sanitized_url = Self::sanitize_ws_url(&config.url);

        // Connect to remote SurrealDB via WebSocket
        let db = Surreal::new::<Ws>(sanitized_url.as_str())
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to SurrealDB at {} (check that server is running and URL is correct)",
                    config.url
                )
            })?;

        // Authenticate with the server
        // If no password is provided, attempt connection without auth (will fail if server requires it)
        if let Some(pass) = &config.pass {
            if verbose {
                eprintln!("[mx] Authenticating as user '{}'", config.user);
            }
            db.signin(Root {
                username: &config.user,
                password: pass,
            })
            .await
            .with_context(|| {
                format!(
                    "Failed to authenticate to SurrealDB at {} as user '{}' (check credentials in MX_SURREAL_USER and MX_SURREAL_PASS)",
                    config.url, config.user
                )
            })?;
        } else if verbose {
            eprintln!("[mx] No password provided, connecting without authentication");
        }

        // Use namespace and database from config
        if verbose {
            eprintln!(
                "[mx] Using namespace '{}' and database '{}'",
                config.namespace, config.database
            );
        }
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .with_context(|| {
                format!(
                    "Failed to set namespace '{}' and database '{}' (check that they exist on the server)",
                    config.namespace, config.database
                )
            })?;

        if verbose {
            eprintln!("[mx] Network connection established successfully");
        }

        // Note: Schema is NOT applied for network mode
        // The remote server should already have the schema
        // (Schema is applied via NixOS module or manual setup)

        Ok(Self {
            conn: SurrealConnection::Network(db),
        })
    }

    /// Legacy async open - kept for compatibility
    async fn open_async<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = SurrealConfig::from_env();
        Self::open_with_config_async(path, &config, false).await
    }

    /// Test helper - open temporary database
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        use tempfile::tempdir;

        let temp_dir = tempdir()?;
        Self::open(temp_dir.path())
    }

    /// Get reference to underlying Surreal instance (embedded only)
    ///
    /// Returns `None` if called on a network connection.
    /// Prefer using connection-agnostic methods instead.
    #[deprecated(note = "Use connection-agnostic methods instead")]
    pub fn inner(&self) -> Option<&Surreal<surrealdb::engine::local::Db>> {
        match &self.conn {
            SurrealConnection::Embedded(db) => Some(db),
            SurrealConnection::Network(_) => None,
        }
    }

    /// Build standard knowledge entry SELECT fields
    fn knowledge_select_fields() -> &'static str {
        "meta::id(id) AS id, title, body, summary, file_path, content_hash, ephemeral,
        owner, visibility,
        meta::id(category) AS category_id,
        meta::id(source_type) AS source_type_id,
        meta::id(entry_type) AS entry_type_id,
        meta::id(content_type) AS content_type_id,
        IF source_project THEN meta::id(source_project) ELSE null END AS source_project_id,
        IF source_agent THEN meta::id(source_agent) ELSE null END AS source_agent_id,
        IF session THEN meta::id(session) ELSE null END AS session_id,
        <string>created_at AS created_at, <string>updated_at AS updated_at,
        IF resonance THEN resonance ELSE 0 END AS resonance,
        resonance_type,
        IF last_activated THEN <string>last_activated ELSE null END AS last_activated,
        IF activation_count THEN activation_count ELSE 0 END AS activation_count,
        IF decay_rate THEN decay_rate ELSE 0.0 END AS decay_rate,
        IF anchors THEN anchors ELSE [] END AS anchors,
        IF wake_phrases THEN wake_phrases ELSE [] END AS wake_phrases,
        wake_order,
        wake_phrase,
        embedding,
        embedding_model,
        IF embedded_at THEN <string>embedded_at ELSE null END AS embedded_at"
    }

    /// Build visibility filter for privacy-aware queries
    fn build_visibility_filter(ctx: &crate::store::AgentContext) -> (String, Option<String>) {
        if ctx.include_private {
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
        }
    }

    /// Build resonance filter clauses
    fn build_resonance_filter(filter: &crate::store::KnowledgeFilter) -> String {
        let mut clauses = Vec::new();

        if let Some(min) = filter.min_resonance {
            clauses.push(format!("resonance >= {}", min));
        }

        if let Some(max) = filter.max_resonance {
            clauses.push(format!("resonance <= {}", max));
        }

        if clauses.is_empty() {
            String::new()
        } else {
            format!("AND ({})", clauses.join(" AND "))
        }
    }

    /// Validate category name to prevent SQL injection
    /// Only allows alphanumeric characters, underscores, and hyphens
    fn is_valid_category_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    /// Build category filter clauses
    /// Category names are validated to prevent SQL injection
    fn build_category_filter(filter: &crate::store::KnowledgeFilter) -> String {
        match &filter.categories {
            Some(cats) if !cats.is_empty() => {
                // Filter out invalid category names to prevent injection
                let valid_cats: Vec<&String> = cats
                    .iter()
                    .filter(|c| Self::is_valid_category_name(c))
                    .collect();

                if valid_cats.is_empty() {
                    return String::new();
                }

                if valid_cats.len() == 1 {
                    format!(
                        "AND category = type::thing('category', '{}')",
                        valid_cats[0]
                    )
                } else {
                    // Multiple categories: use IN clause
                    let quoted: Vec<String> = valid_cats
                        .iter()
                        .map(|c| format!("type::thing('category', '{}')", c))
                        .collect();
                    format!("AND category IN [{}]", quoted.join(", "))
                }
            }
            _ => String::new(),
        }
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
            content_type = type::thing('content_type', $content_type_id),
            resonance = $resonance,
            resonance_type = $resonance_type,
            activation_count = $activation_count,
            decay_rate = $decay_rate,
            anchors = $anchors,
            wake_phrases = $wake_phrases,
            wake_order = $wake_order,
            wake_phrase = $wake_phrase,
            embedding = $embedding,
            embedding_model = $embedding_model"
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
        if entry.last_activated.is_some() {
            query.push_str(", last_activated = <datetime>$last_activated");
        }
        if entry.embedded_at.is_some() {
            query.push_str(", embedded_at = <datetime>$embedded_at");
        }

        // Bind all parameters and execute query
        let mut response = with_db!(self, db, {
            let mut q = db
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
                ))
                .bind(("resonance", entry.resonance))
                .bind(("resonance_type", entry.resonance_type.clone()))
                .bind(("activation_count", entry.activation_count))
                .bind(("decay_rate", entry.decay_rate))
                .bind(("anchors", entry.anchors.clone()))
                .bind(("wake_phrases", entry.wake_phrases.clone()))
                .bind(("wake_order", entry.wake_order))
                .bind(("wake_phrase", entry.wake_phrase.clone()))
                .bind(("embedding", entry.embedding.clone()))
                .bind(("embedding_model", entry.embedding_model.clone()));

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
            if let Some(ref activated) = entry.last_activated {
                q = q.bind(("last_activated", normalize_datetime(activated)));
            }
            if let Some(ref embedded) = entry.embedded_at {
                q = q.bind(("embedded_at", normalize_datetime(embedded)));
            }

            q.await.context("Failed to upsert knowledge record")
        })?;

        // Check for errors in the response
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("SurrealDB returned errors: {:?}", errors));
        }

        // Manage tags - delete old, create new
        let mut tag_delete_response = with_db!(self, db, {
            db.query("DELETE tagged_with WHERE in = $knowledge")
                .bind(("knowledge", record_id.0.clone()))
                .await
                .context("Failed to clear existing tags")
        })?;

        let tag_delete_errors = tag_delete_response.take_errors();
        if !tag_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                tag_delete_errors
            ));
        }

        for tag_name in &entry.tags {
            // Ensure tag exists - use query UPSERT to handle schema defaults
            let mut tag_response = with_db!(self, db, {
                db.query("UPSERT type::thing('tag', $tag_id) SET name = $tag_name")
                    .bind(("tag_id", tag_name.clone()))
                    .bind(("tag_name", tag_name.clone()))
                    .await
                    .context("Failed to create tag")
            })?;

            let tag_errors = tag_response.take_errors();
            if !tag_errors.is_empty() {
                return Err(anyhow::anyhow!("Failed to create tag: {:?}", tag_errors));
            }

            let tag_id = RecordId::new("tag", tag_name);

            // Create edge
            let mut tag_edge_response = with_db!(self, db, {
                db.query("RELATE $knowledge->tagged_with->$tag")
                    .bind(("knowledge", record_id.0.clone()))
                    .bind(("tag", tag_id.0.clone()))
                    .await
                    .context("Failed to create tag edge")
            })?;

            let tag_edge_errors = tag_edge_response.take_errors();
            if !tag_edge_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "SurrealDB returned errors: {:?}",
                    tag_edge_errors
                ));
            }
        }

        // Manage applicability - delete old, create new
        let mut app_delete_response = with_db!(self, db, {
            db.query("DELETE applies_to WHERE in = $knowledge")
                .bind(("knowledge", record_id.0.clone()))
                .await
                .context("Failed to clear existing applicability")
        })?;

        let app_delete_errors = app_delete_response.take_errors();
        if !app_delete_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "SurrealDB returned errors: {:?}",
                app_delete_errors
            ));
        }

        for app_type in &entry.applicability {
            // Ensure applicability_type exists - use query UPSERT to handle schema defaults
            let mut app_type_response = with_db!(self, db, {
                db.query("UPSERT type::thing('applicability_type', $app_type_id) SET description = $app_type_desc")
                    .bind(("app_type_id", app_type.clone()))
                    .bind(("app_type_desc", format!("Applicability: {}", app_type)))
                    .await
                    .context("Failed to create applicability_type")
            })?;

            let app_type_errors = app_type_response.take_errors();
            if !app_type_errors.is_empty() {
                return Err(anyhow::anyhow!(
                    "Failed to create applicability_type: {:?}",
                    app_type_errors
                ));
            }

            let app_id = RecordId::new("applicability_type", app_type);

            // Create edge
            let mut app_edge_response = with_db!(self, db, {
                db.query("RELATE $knowledge->applies_to->$app_type")
                    .bind(("knowledge", record_id.0.clone()))
                    .bind(("app_type", app_id.0.clone()))
                    .await
                    .context("Failed to create applicability edge")
            })?;

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

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE meta::id(id) = $id {}",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("id", id_part.to_string()));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query knowledge record")
        })?;

        // Direct deserialization to DTO - no manual JSON parsing!
        let records: Vec<SurrealKnowledgeRecord> = response.take(0)?;

        if records.is_empty() {
            return Ok(None);
        }

        let record = records.into_iter().next().unwrap();

        // Fetch tags and applicability separately
        let tags = self
            .get_tags_for_entry_async(&format!("kn-{}", record.id))
            .await?;
        let applicability = self
            .get_applicability_for_entry_async(&format!("kn-{}", record.id))
            .await?;

        Ok(Some(record.into_knowledge_entry(tags, applicability)))
    }

    /// Delete a knowledge entry (edges cascade automatically)
    pub fn delete_knowledge(&self, id: &str) -> Result<bool> {
        Self::runtime().block_on(self.delete_knowledge_async(id))
    }

    async fn delete_knowledge_async(&self, id: &str) -> Result<bool> {
        let id_part = id.strip_prefix("kn-").unwrap_or(id);

        // First check if the record exists
        let mut check_response = with_db!(self, db, {
            db.query("SELECT count() AS c FROM knowledge WHERE meta::id(id) = $id GROUP ALL")
                .bind(("id", id_part.to_string()))
                .await
                .context("Failed to check knowledge record existence")
        })?;

        let count_results: Vec<serde_json::Value> = check_response.take(0)?;
        let exists = count_results
            .first()
            .and_then(|v| v["c"].as_i64())
            .unwrap_or(0)
            > 0;

        if !exists {
            return Ok(false);
        }

        // Delete without RETURN to avoid deserialization issues with Thing fields
        // The knowledge table has many Thing fields (category, source_type, etc) that
        // surrealdb::sql::Value cannot deserialize
        let mut response = with_db!(self, db, {
            db.query("DELETE type::thing('knowledge', $id)")
                .bind(("id", id_part.to_string()))
                .await
                .context("Failed to delete knowledge record")
        })?;

        // Check for errors
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!("Delete failed: {:?}", errors));
        }

        Ok(true)
    }

    /// Search knowledge using BM25 full-text indexes
    pub fn search_knowledge(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.search_knowledge_async(query, ctx, filter))
    }

    async fn search_knowledge_async(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        let query_owned = query.to_string();

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);
        let category_clause = Self::build_category_filter(filter);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE (title @@ $query OR body @@ $query OR summary @@ $query) {} {} {}",
            Self::knowledge_select_fields(),
            visibility_clause,
            resonance_clause,
            category_clause
        );

        let mut response = with_db!(self, db, {
            let mut query_builder = db.query(&sql).bind(("query", query_owned));
            if let Some(agent) = current_agent {
                query_builder = query_builder.bind(("current_agent", agent));
            }
            query_builder
                .await
                .context("Failed to execute search query")
        })?;

        let results: Vec<serde_json::Value> =
            response.take(0).context("Failed to parse search results")?;

        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Semantic search using vector similarity (brute force cosine)
    pub fn semantic_search_knowledge(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.semantic_search_knowledge_async(
            query_embedding,
            ctx,
            filter,
            limit,
        ))
    }

    async fn semantic_search_knowledge_async(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);
        let category_clause = Self::build_category_filter(filter);

        // Brute force vector similarity search (no HNSW index)
        let sql = format!(
            "SELECT {}, vector::similarity::cosine(embedding, $query_vec) AS score
            FROM knowledge
            WHERE embedding IS NOT NONE {} {} {}
            ORDER BY score DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause,
            resonance_clause,
            category_clause
        );

        let mut response = with_db!(self, db, {
            let mut query_builder = db
                .query(&sql)
                .bind(("query_vec", query_embedding.to_vec()))
                .bind(("limit", limit));
            if let Some(agent) = current_agent {
                query_builder = query_builder.bind(("current_agent", agent));
            }
            query_builder
                .await
                .context("Failed to execute semantic search query")
        })?;

        let results: Vec<serde_json::Value> = response
            .take(0)
            .context("Failed to parse semantic search results")?;

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
        let mut tags_response = with_db!(self, db, {
            db.query("SELECT VALUE out.name FROM tagged_with WHERE in = $knowledge")
                .bind(("knowledge", knowledge_thing.clone()))
                .await
                .context("Failed to query tags")
        })?;
        let tags: Vec<String> = tags_response.take(0).unwrap_or_default();

        // Fetch applicability
        let mut app_response = with_db!(self, db, {
            db.query("SELECT VALUE meta::id(out) FROM applies_to WHERE in = $knowledge")
                .bind(("knowledge", knowledge_thing))
                .await
                .context("Failed to query applicability")
        })?;
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
            resonance: serde_json::from_value(obj["resonance"].clone()).unwrap_or(0),
            resonance_type: serde_json::from_value(obj["resonance_type"].clone()).ok(),
            last_activated: serde_json::from_value(obj["last_activated"].clone()).ok(),
            activation_count: serde_json::from_value(obj["activation_count"].clone()).unwrap_or(0),
            decay_rate: serde_json::from_value(obj["decay_rate"].clone()).unwrap_or(0.0),
            anchors: serde_json::from_value(obj["anchors"].clone()).unwrap_or_default(),
            wake_phrases: serde_json::from_value(obj["wake_phrases"].clone()).unwrap_or_default(),
            wake_order: serde_json::from_value(obj["wake_order"].clone()).ok(),
            wake_phrase: serde_json::from_value(obj["wake_phrase"].clone()).ok(),
            embedding: serde_json::from_value(obj["embedding"].clone()).ok(),
            embedding_model: serde_json::from_value(obj["embedding_model"].clone()).ok(),
            embedded_at: serde_json::from_value(obj["embedded_at"].clone()).ok(),
        })
    }

    // =========================================================================
    // WAKE CASCADE - Three-layer resonance query for identity loading
    // =========================================================================

    /// Wake-up cascade: Load Q's identity through three layers of resonance
    pub fn wake_cascade(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        Self::runtime().block_on(self.wake_cascade_async(ctx, limit, min_resonance, days))
    }

    async fn wake_cascade_async(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        // If min_resonance is set, use simple query for all blooms >= threshold
        if let Some(threshold) = min_resonance {
            let blooms = self.query_blooms_by_resonance(ctx, threshold).await?;
            return Ok(crate::store::WakeCascade {
                core: blooms,
                recent: Vec::new(),
                bridges: Vec::new(),
            });
        }

        // Sequential filling: core first, then recent, then bridges
        // This ensures we get the most important blooms first

        // Layer 1: Core foundational/transformative blooms (resonance 8+)
        // Use full limit for core - we'll subtract what we get
        let core = self.query_core_blooms(ctx, limit).await?;
        let remaining = limit.saturating_sub(core.len());

        // Layer 2: Recent blooms (last N days)
        // Exclude IDs already in core, use remaining quota
        let core_ids: std::collections::HashSet<String> =
            core.iter().map(|e| e.id.clone()).collect();

        let all_recent = self.query_recent_blooms(ctx, remaining * 2, days).await?;
        let recent: Vec<_> = all_recent
            .into_iter()
            .filter(|e| !core_ids.contains(&e.id))
            .take(remaining)
            .collect();
        let remaining = remaining.saturating_sub(recent.len());

        // Layer 3: Bridge blooms (anchored to core/recent, resonance 5+)
        // Use final remaining quota
        let mut anchor_ids: Vec<String> = core
            .iter()
            .chain(recent.iter())
            .map(|e| e.id.strip_prefix("kn-").unwrap_or(&e.id).to_string())
            .collect();

        // Deduplicate anchor IDs
        anchor_ids.sort();
        anchor_ids.dedup();

        let bridges = if anchor_ids.is_empty() || remaining == 0 {
            Vec::new()
        } else {
            // Exclude IDs already in core/recent
            let mut existing_ids = core_ids;
            existing_ids.extend(recent.iter().map(|e| e.id.clone()));

            let all_bridges = self
                .query_bridge_blooms(ctx, remaining * 2, &anchor_ids)
                .await?;
            all_bridges
                .into_iter()
                .filter(|e| !existing_ids.contains(&e.id))
                .take(remaining)
                .collect()
        };

        Ok(crate::store::WakeCascade {
            core,
            recent,
            bridges,
        })
    }

    /// Query all blooms with resonance >= threshold (for --min-resonance flag)
    async fn query_blooms_by_resonance(
        &self,
        ctx: &crate::store::AgentContext,
        threshold: i32,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE resonance >= $threshold
            {}
            ORDER BY resonance DESC",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("threshold", threshold));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query blooms by resonance")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 1: Query core blooms (resonance 8+, foundational/transformative)
    async fn query_core_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE resonance >= 8
                AND resonance_type IN ['foundational', 'transformative']
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query core blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 2: Query recent blooms (last N days, sorted by resonance)
    async fn query_recent_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        days: i64,
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Calculate cutoff date (N days ago)
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        let cutoff_str = cutoff.to_rfc3339();

        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE last_activated > <datetime>$cutoff
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db
                .query(&sql)
                .bind(("cutoff", cutoff_str))
                .bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query recent blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Layer 3: Query bridge blooms (anchored to core/recent, resonance 5+)
    async fn query_bridge_blooms(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        anchor_ids: &[String],
    ) -> Result<Vec<crate::knowledge::KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Use array::intersect to check if anchors array has any overlap with anchor_ids
        // If intersection is non-empty, this bloom is anchored to a core/recent bloom
        let sql = format!(
            "SELECT *,
                (wake_order IS NOT NULL) AS has_wake_order,
                wake_order ?? 999999 AS effective_wake_order
            FROM (
                SELECT {}
                FROM knowledge
                WHERE array::len(array::intersect(anchors, $anchor_ids)) > 0
                AND resonance >= 5
                {}
            )
            ORDER BY
                has_wake_order DESC,
                effective_wake_order ASC,
                resonance DESC
            LIMIT $limit",
            Self::knowledge_select_fields(),
            visibility_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db
                .query(&sql)
                .bind(("anchor_ids", anchor_ids.to_vec()))
                .bind(("limit", limit as i64));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query bridge blooms")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();
        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// Update activation counts for loaded blooms
    pub fn update_activations(&self, ids: &[String]) -> Result<()> {
        Self::runtime().block_on(self.update_activations_async(ids))
    }

    async fn update_activations_async(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        // Strip "kn-" prefix from IDs if present
        let clean_ids: Vec<String> = ids
            .iter()
            .map(|id| id.strip_prefix("kn-").unwrap_or(id).to_string())
            .collect();

        // Build array of Thing references
        let things: Vec<Thing> = clean_ids
            .iter()
            .map(|id| Thing::from(("knowledge", id.as_str())))
            .collect();

        let mut response = with_db!(self, db, {
            db.query(
                "UPDATE knowledge SET
                activation_count += 1,
                last_activated = time::now()
                WHERE id IN $ids",
            )
            .bind(("ids", things))
            .await
            .context("Failed to update activations")
        })?;

        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to update activations: {:?}",
                errors
            ));
        }

        Ok(())
    }

    // =========================================================================
    // LOOKUP OPERATIONS
    // =========================================================================

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

        with_db!(self, db, {
            db.query("RELATE $from->relates_to->$to SET relationship_type = $rel_type")
                .bind(("from", from_thing))
                .bind(("to", to_thing))
                .bind(("rel_type", rel_type_thing))
                .await
                .context("Failed to create relationship")
        })?;

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
        let mut response = with_db!(self, db, {
            db.query(
                "SELECT id, in AS from_entry_id, out AS to_entry_id, relationship_type, <string>created_at AS created_at
                 FROM relates_to
                 WHERE in = $entry OR out = $entry
                 ORDER BY created_at DESC"
            )
            .bind(("entry", entry_thing))
            .await
            .context("Failed to query relationships")
        })?;

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

        let mut response = with_db!(self, db, {
            db.query(
                "DELETE relates_to
                 WHERE in = $from AND out = $to AND relationship_type = $rel_type
                 RETURN BEFORE",
            )
            .bind(("from", from_thing))
            .bind(("to", to_thing))
            .bind(("rel_type", rel_type_thing))
            .await
            .context("Failed to delete relationship")
        })?;

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

        let mut tags_response = with_db!(self, db, {
            db.query("SELECT VALUE out.name FROM tagged_with WHERE in = $knowledge")
                .bind(("knowledge", entry_thing))
                .await
                .context("Failed to query tags")
        })?;

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

        let mut app_response = with_db!(self, db, {
            db.query("SELECT VALUE meta::id(out) FROM applies_to WHERE in = $knowledge")
                .bind(("knowledge", entry_thing))
                .await
                .context("Failed to query applicability")
        })?;

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
        let result: Option<Value> = with_db!(self, db, {
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

    /// List tables - SurrealDB uses tables, return table names
    pub fn list_tables(&self) -> Result<Vec<String>> {
        Self::runtime().block_on(self.list_tables_async())
    }

    async fn list_tables_async(&self) -> Result<Vec<String>> {
        let mut response = with_db!(self, db, {
            db.query("INFO FOR DB")
                .await
                .context("Failed to query database info")
        })?;

        // SurrealDB INFO returns complex metadata - take as JSON directly
        let info: Option<serde_json::Value> = response.take(0)?;
        let mut tables = Vec::new();

        if let Some(info_json) = info
            && let Some(tables_obj) = info_json.get("tables").and_then(|v| v.as_object())
        {
            for table_name in tables_obj.keys() {
                tables.push(table_name.clone());
            }
            tables.sort();
        }

        Ok(tables)
    }

    /// Count total knowledge entries
    pub fn count(&self) -> Result<usize> {
        Self::runtime().block_on(self.count_async())
    }

    async fn count_async(&self) -> Result<usize> {
        let mut response = with_db!(self, db, {
            db.query("SELECT count() AS c FROM knowledge GROUP ALL")
                .await
                .context("Failed to count knowledge entries")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let count = results.first().and_then(|v| v["c"].as_i64()).unwrap_or(0) as usize;
        Ok(count)
    }

    /// List all knowledge entries
    pub fn list_all(&self, ctx: &crate::store::AgentContext) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_all_async(ctx))
    }

    async fn list_all_async(
        &self,
        ctx: &crate::store::AgentContext,
    ) -> Result<Vec<KnowledgeEntry>> {
        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);

        // Convert AND to WHERE for list_all (no WHERE clause exists yet)
        let where_clause = visibility_clause.replacen("AND", "WHERE", 1);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            {}
            ORDER BY title",
            Self::knowledge_select_fields(),
            where_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql);
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query all knowledge entries")
        })?;

        let results: Vec<serde_json::Value> = response.take(0)?;
        let mut entries = Vec::new();

        for obj in results {
            entries.push(self.value_to_knowledge_entry(obj).await?);
        }

        Ok(entries)
    }

    /// List entries by category
    pub fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        Self::runtime().block_on(self.list_by_category_async(category, ctx, filter))
    }

    async fn list_by_category_async(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        let category_thing = Thing::from(("category", category));

        let (visibility_clause, current_agent) = Self::build_visibility_filter(ctx);
        let resonance_clause = Self::build_resonance_filter(filter);

        let sql = format!(
            "SELECT {}
            FROM knowledge
            WHERE category = $category {} {}
            ORDER BY title",
            Self::knowledge_select_fields(),
            visibility_clause,
            resonance_clause
        );

        let mut response = with_db!(self, db, {
            let mut query = db.query(&sql).bind(("category", category_thing));
            if let Some(agent) = current_agent {
                query = query.bind(("current_agent", agent));
            }
            query.await.context("Failed to query knowledge by category")
        })?;

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

    fn search(
        &self,
        query: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.search_knowledge(query, ctx, filter)
    }

    fn semantic_search(
        &self,
        query_embedding: &[f32],
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.semantic_search_knowledge(query_embedding, ctx, filter, limit)
    }

    fn list_by_category(
        &self,
        category: &str,
        ctx: &crate::store::AgentContext,
        filter: &crate::store::KnowledgeFilter,
    ) -> Result<Vec<KnowledgeEntry>> {
        self.list_by_category(category, ctx, filter)
    }

    fn list_all(&self, ctx: &crate::store::AgentContext) -> Result<Vec<KnowledgeEntry>> {
        self.list_all(ctx)
    }

    fn count(&self) -> Result<usize> {
        self.count()
    }

    fn wake_cascade(
        &self,
        ctx: &crate::store::AgentContext,
        limit: usize,
        min_resonance: Option<i32>,
        days: i64,
    ) -> Result<crate::store::WakeCascade> {
        self.wake_cascade(ctx, limit, min_resonance, days)
    }

    fn update_activations(&self, ids: &[String]) -> Result<()> {
        self.update_activations(ids)
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
