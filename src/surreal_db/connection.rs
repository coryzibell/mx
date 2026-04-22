use anyhow::{Context, Result};
use std::path::Path;
use std::sync::OnceLock;
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb::engine::remote::ws::{Client as WsClient, Ws, Wss};
use surrealdb::opt::auth::{Database, Namespace, Root};
use tokio::runtime::Runtime;

use super::SurrealDatabase;

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

/// Authentication level for SurrealDB signin
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AuthLevel {
    /// Root-level authentication (default)
    #[default]
    Root,
    /// Namespace-level authentication
    Namespace,
    /// Database-level authentication
    Database,
}

impl AuthLevel {
    /// Parse an auth level from an environment variable string.
    ///
    /// Accepts lowercase aliases:
    /// - "root" -> Root
    /// - "namespace" or "ns" -> Namespace
    /// - "database" or "db" -> Database
    pub fn from_env_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "root" => Ok(Self::Root),
            "namespace" | "ns" => Ok(Self::Namespace),
            "database" | "db" => Ok(Self::Database),
            other => anyhow::bail!(
                "Unknown MX_SURREAL_AUTH_LEVEL '{}'. Valid values: root, namespace (ns), database (db)",
                other
            ),
        }
    }
}

impl std::fmt::Display for AuthLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root => write!(f, "root"),
            Self::Namespace => write!(f, "namespace"),
            Self::Database => write!(f, "database"),
        }
    }
}

/// Configuration for SurrealDB connection
///
/// Parsed from environment variables:
/// - `MX_SURREAL_MODE`: "embedded" (default) or "network"
/// - `MX_SURREAL_URL`: WebSocket URL for network mode (default: ws://localhost:8000)
/// - `MX_SURREAL_USER`: Username for network auth (default: root)
/// - `MX_SURREAL_PASS`: Password for network auth (direct value)
/// - `MX_SURREAL_PASS_FILE`: Path to file containing password (e.g., agenix secret)
/// - `MX_SURREAL_AUTH_LEVEL`: Auth level for signin: "root" (default), "namespace"/"ns", or "database"/"db"
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
    /// Auth level for signin (root, namespace, or database)
    pub auth_level: AuthLevel,
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
            auth_level: AuthLevel::Root,
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

        let auth_level_str =
            std::env::var("MX_SURREAL_AUTH_LEVEL").unwrap_or_else(|_| "root".to_string());
        let auth_level = AuthLevel::from_env_str(&auth_level_str).unwrap_or_else(|e| {
            eprintln!("[mx] WARNING: {e}, defaulting to root");
            AuthLevel::Root
        });

        Self {
            mode,
            url,
            user,
            pass,
            namespace,
            database,
            auth_level,
        }
    }

    /// Check if we're in network mode
    pub fn is_network(&self) -> bool {
        self.mode == SurrealMode::Network
    }
}

/// Connection abstraction for SurrealDB - supports both embedded and network modes
pub enum SurrealConnection {
    /// Embedded SurrealKV database (local file-based)
    Embedded(Surreal<surrealdb::engine::local::Db>),
    /// Network connection via WebSocket
    Network(Surreal<WsClient>),
}

/// Embedded SurrealDB schema - applied on database open
const SCHEMA: &str = include_str!("../../schema/surrealdb-schema.surql");

/// Normalize datetime string to RFC3339 format for SurrealDB
pub(super) fn normalize_datetime(s: &str) -> String {
    // If already looks like RFC3339 (has T and timezone), return as-is
    if s.contains('T') && (s.ends_with('Z') || s.contains('+') || s.contains("-0")) {
        return s.to_string();
    }

    // Space-separated format: "2025-11-29 08:10:33" -> "2025-11-29T08:10:33Z"
    if s.contains(' ') && !s.contains('T') {
        return s.replace(' ', "T") + "Z";
    }

    // Fallback: assume it's already good or add Z
    if !s.ends_with('Z') && !s.contains('+') {
        return format!("{}Z", s);
    }

    s.to_string()
}

// ============================================================================
// CONNECTION METHODS ON SurrealDatabase
// ============================================================================

impl SurrealDatabase {
    /// Get or initialize the global tokio runtime
    pub(super) fn runtime() -> &'static Runtime {
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

        // Strip protocol prefix from URL - surrealdb crate expects just host:port.
        // We lose the scheme in the sanitized form, so detect it up front to
        // pick the right engine (plain Ws vs TLS Wss).
        let is_tls = config.url.starts_with("wss://");
        let sanitized_url = Self::sanitize_ws_url(&config.url);

        // Connect to remote SurrealDB via WebSocket. Dispatch on scheme:
        // `Ws` speaks plain WebSocket; `Wss` is the TLS variant (enabled by
        // the `rustls` feature on the surrealdb crate). Using the wrong one
        // fails with a cryptic "HTTP version must be 1.1 or higher" because
        // the TLS handshake bytes get parsed as HTTP.
        let db = if is_tls {
            Surreal::new::<Wss>(sanitized_url.as_str())
                .await
                .with_context(|| {
                    format!(
                        "Failed to connect to SurrealDB at {} (check that server is running and URL is correct)",
                        config.url
                    )
                })?
        } else {
            Surreal::new::<Ws>(sanitized_url.as_str())
                .await
                .with_context(|| {
                    format!(
                        "Failed to connect to SurrealDB at {} (check that server is running and URL is correct)",
                        config.url
                    )
                })?
        };

        // Authenticate with the server
        // If no password is provided, attempt connection without auth (will fail if server requires it)
        if let Some(pass) = &config.pass {
            if verbose {
                eprintln!(
                    "[mx] Authenticating as user '{}' (auth level: {})",
                    config.user, config.auth_level
                );
            }
            match config.auth_level {
                AuthLevel::Namespace => {
                    db.signin(Namespace {
                        namespace: &config.namespace,
                        username: &config.user,
                        password: pass,
                    })
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to authenticate to SurrealDB at {} as namespace-level user '{}' in namespace '{}' (check credentials in MX_SURREAL_USER and MX_SURREAL_PASS)",
                            config.url, config.user, config.namespace
                        )
                    })?;
                }
                AuthLevel::Database => {
                    db.signin(Database {
                        namespace: &config.namespace,
                        database: &config.database,
                        username: &config.user,
                        password: pass,
                    })
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to authenticate to SurrealDB at {} as database-level user '{}' in namespace '{}' database '{}' (check credentials in MX_SURREAL_USER and MX_SURREAL_PASS)",
                            config.url, config.user, config.namespace, config.database
                        )
                    })?;
                }
                AuthLevel::Root => {
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
                }
            }
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
    #[allow(dead_code)]
    async fn open_async<P: AsRef<Path>>(path: P) -> Result<Self> {
        let config = SurrealConfig::from_env();
        Self::open_with_config_async(path, &config, false).await
    }

    /// Test helper - open temporary database
    ///
    /// Forces embedded mode regardless of `MX_SURREAL_MODE` env var,
    /// so tests never hit the live database.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        use tempfile::tempdir;

        let temp_dir = tempdir()?;
        let config = SurrealConfig::default(); // always Embedded
        Self::connect(temp_dir.path(), &config)
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
}
