//! Lightweight local KV store for fast agent state.
//!
//! Backed by a TOML schema file and a JSON data file. All writes are atomic
//! (serialize to tmp, fsync, rename). No networking, no database.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use base_d::{DictionaryRegistry, HashAlgorithm, encode, hash};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::cli::TimeRangeArgs;

/// Per-process flag so the legacy `~/.crewu/kv/` warning prints at most once,
/// even if both schema and data fall back to the legacy location.
static LEGACY_KV_WARNING_EMITTED: OnceLock<()> = OnceLock::new();

/// Cached dictionary registry for hash generation -- avoids re-allocating
/// the HashMap on every `generate_entry_hash` call.
static DICT_REGISTRY: OnceLock<DictionaryRegistry> = OnceLock::new();

/// The single legacy-fallback warning copy. Lives here so the schema and data
/// resolvers cannot drift apart.
const LEGACY_KV_WARNING: &str = "note: reading kv from `~/.crewu/kv/` -- this default is moving to \
     `$MX_HOME/kv/` in a future release. Move your files or set \
     `MX_KV_SCHEMA` / `MX_KV_DATA`.";

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

/// Pure decision: should the consolidated legacy-warning be emitted now?
///
/// Returns true exactly once across the lifetime of the supplied `gate`,
/// and only when at least one of the resolvers reported a legacy fallback.
/// Lifting this out of `from_env` keeps the dedupe logic unit-testable
/// without touching process-global stderr or env vars.
pub(crate) fn should_emit_legacy_kv_warning(
    schema_warn: bool,
    data_warn: bool,
    gate: &OnceLock<()>,
) -> bool {
    if !(schema_warn || data_warn) {
        return false;
    }
    // `set` returns Ok the first time, Err every time after.
    gate.set(()).is_ok()
}

/// Pure path-resolution helper for kv schema/data files.
///
/// Returns `(resolved_path, should_warn)`. `should_warn` is true only when
/// the legacy `~/.crewu/kv/` location is being used as a soft fallback.
///
/// Resolution order:
/// 1. Env override (with `{agent}` placeholder substitution); empty value
///    is treated as unset
/// 2. New `$MX_HOME/kv/...` location (if file exists)
/// 3. Legacy `~/.crewu/kv/...` (if file exists) -- emits warning
/// 4. Otherwise, return the new location (so error messages point at the
///    canonical place)
///
/// TODO(kv-path-migration): drop the legacy fallback after one release cycle.
pub(crate) fn resolve_kv_path_with(
    env_val: Option<&str>,
    agent: &str,
    new_default: PathBuf,
    legacy: Option<PathBuf>,
) -> (PathBuf, bool) {
    if let Some(p) = env_val
        && !p.is_empty()
    {
        return (PathBuf::from(p.replace("{agent}", agent)), false);
    }

    if new_default.exists() {
        return (new_default, false);
    }

    if let Some(legacy) = legacy
        && legacy.exists()
    {
        return (legacy, true);
    }

    (new_default, false)
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

pub const EXIT_OK: i32 = 0;
pub const EXIT_KEY_NOT_FOUND: i32 = 1;
pub const EXIT_TYPE_MISMATCH: i32 = 2;
pub const EXIT_SCHEMA_MISSING: i32 = 3;
pub const EXIT_INVALID_INPUT: i32 = 4;

// ---------------------------------------------------------------------------
// Typed errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum KvError {
    KeyNotFound(String),
    TypeMismatch {
        key: String,
        expected: String,
        got: String,
    },
    SchemaMissing(PathBuf),
    EntryNotFound {
        key: String,
        id: String,
    },
    AmbiguousHash {
        prefix: String,
        count: usize,
    },
    Other(anyhow::Error),
}

impl std::fmt::Display for KvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KvError::KeyNotFound(key) => write!(f, "Unknown key: {}", key),
            KvError::TypeMismatch { key, expected, got } => {
                write!(
                    f,
                    "Type mismatch: key '{}' is {}, not {}",
                    key, got, expected
                )
            }
            KvError::SchemaMissing(path) => {
                write!(f, "Schema file not found: {}", path.display())
            }
            KvError::EntryNotFound { key, id } => {
                write!(f, "Entry not found: ID {} in key '{}'", id, key)
            }
            KvError::AmbiguousHash { prefix, count } => {
                write!(
                    f,
                    "hash prefix 'kv-{}' is ambiguous: matches {} entries, provide more characters",
                    prefix, count
                )
            }
            KvError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for KvError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KvError::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for KvError {
    fn from(e: anyhow::Error) -> Self {
        KvError::Other(e)
    }
}

// ---------------------------------------------------------------------------
// Schema types (parsed from TOML)
// ---------------------------------------------------------------------------

/// Top-level schema definition.
#[derive(Debug, Deserialize)]
pub struct Schema {
    pub keys: BTreeMap<String, KeyDef>,
}

/// Definition of a single key in the schema.
#[derive(Debug, Deserialize, Clone)]
pub struct KeyDef {
    #[serde(rename = "type")]
    pub value_type: ValueType,

    #[serde(default)]
    pub min: Option<i64>,

    #[serde(default)]
    pub max: Option<i64>,

    #[serde(default)]
    pub default: Option<String>,

    #[serde(default)]
    pub max_entries: Option<usize>,

    #[serde(default)]
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    Counter,
    History,
    State,
    String,
    List,
}

impl std::fmt::Display for ValueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueType::Counter => write!(f, "counter"),
            ValueType::History => write!(f, "history"),
            ValueType::State => write!(f, "state"),
            ValueType::String => write!(f, "string"),
            ValueType::List => write!(f, "list"),
        }
    }
}

// ---------------------------------------------------------------------------
// Data types (persisted as JSON)
// ---------------------------------------------------------------------------

/// Top-level data file.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DataFile {
    #[serde(rename = "_schema")]
    pub schema_id: String,

    #[serde(rename = "_updated")]
    pub updated: String,

    #[serde(flatten)]
    pub entries: BTreeMap<String, DataValue>,
}

/// A single stored value.
///
/// Deserialization note: we use a custom Deserialize impl to handle backward
/// compatibility for lists (old format: `items: ["a","b"]`, new format:
/// `items: [{id,value,ts}, ...]`) and for history entries that may lack `id`.
#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
pub enum DataValue {
    Counter {
        value: i64,
    },
    String {
        value: String,
    },
    History {
        entries: Vec<HistoryEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<String>,
    },
    State {
        fields: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<String>,
    },
    List {
        items: Vec<ListEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Backward-compatible deserialization
// ---------------------------------------------------------------------------

/// Intermediate type that can accept either old bare-string lists or new ListEntry lists.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawListItem {
    Entry(ListEntry),
    Bare(String),
}

/// Mirror of DataValue for deserialization, with backward compat glue.
///
/// The `data` field on `HistoryEntry` and `ListEntry` was added after initial
/// release. Backward compatibility for files written before that field existed
/// is handled by `#[serde(default)]` on each entry struct's `data` field, so
/// `DataValueDe` itself needs no special handling for it.
#[derive(Deserialize)]
#[serde(untagged)]
enum DataValueDe {
    Counter {
        value: i64,
    },
    History {
        entries: Vec<HistoryEntry>,
        #[serde(default)]
        memory: Option<String>,
    },
    State {
        fields: BTreeMap<String, String>,
        #[serde(default)]
        memory: Option<String>,
    },
    List {
        items: Vec<RawListItem>,
        #[serde(default)]
        memory: Option<String>,
    },
    // String must be last in untagged — it's the broadest match
    String {
        value: String,
    },
}

impl<'de> Deserialize<'de> for DataValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = DataValueDe::deserialize(deserializer)?;
        Ok(match raw {
            DataValueDe::Counter { value } => DataValue::Counter { value },
            DataValueDe::String { value } => DataValue::String { value },
            DataValueDe::History { entries, memory } => {
                // Back-fill IDs for history entries loaded with id=0 (old data)
                let mut max_id = entries.iter().map(|e| e.id).max().unwrap_or(0);
                let entries = entries
                    .into_iter()
                    .map(|mut e| {
                        if e.id == 0 {
                            max_id += 1;
                            e.id = max_id;
                        }
                        e
                    })
                    .collect();
                DataValue::History { entries, memory }
            }
            DataValueDe::State { fields, memory } => DataValue::State { fields, memory },
            DataValueDe::List { items, memory } => {
                let mut next_id = 0u64;
                let entries: Vec<ListEntry> = items
                    .into_iter()
                    .map(|item| match item {
                        RawListItem::Entry(e) => {
                            if e.id >= next_id {
                                next_id = e.id + 1;
                            }
                            e
                        }
                        RawListItem::Bare(s) => {
                            next_id += 1;
                            ListEntry {
                                id: next_id,
                                hash: String::new(),
                                value: s,
                                ts: String::new(),
                                data: None,
                                memory: None,
                            }
                        }
                    })
                    .collect();
                DataValue::List {
                    items: entries,
                    memory,
                }
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub hash: String,
    pub value: String,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ListEntry {
    pub id: u64,
    #[serde(default)]
    pub hash: String,
    pub value: String,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// Remove result for the CLI layer to format output.
#[derive(Debug)]
pub struct RemoveResult {
    pub removed: Vec<String>,
}

/// Search result entry.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: u64,
    pub hash: String,
    pub value: String,
    pub ts: String,
    pub data: Option<serde_json::Value>,
    pub memory: Option<String>,
}

/// Count result.
#[derive(Debug)]
pub struct CountResult {
    /// Number of entries that matched (all entries when no filter, matching entries when filtered).
    pub matched: usize,
    /// Total entries for the key. Present only when a value filter was applied.
    pub total: Option<usize>,
    pub latest_ts: Option<String>,
}

// ---------------------------------------------------------------------------
// Hash ID generation
// ---------------------------------------------------------------------------

/// Generate a short, stable base58 hash for a kv entry.
///
/// Input: `"{key}:{ts}:{id}"` hashed with blake3, first 4 bytes encoded as
/// base58.  Produces a ~5-6 character alphanumeric string.
pub fn generate_entry_hash(key: &str, ts: &str, id: u64) -> String {
    let input = format!("{}:{}:{}", key, ts, id);
    let hash_bytes = hash(input.as_bytes(), HashAlgorithm::Blake3);
    let registry = DICT_REGISTRY
        .get_or_init(|| DictionaryRegistry::load_default().expect("base-d dictionaries"));
    let dict = registry.dictionary("base58").expect("base58 dictionary");
    encode(&hash_bytes[..4], &dict)
}

/// Result of a push operation, carrying both the numeric and hash IDs.
#[derive(Debug, Clone)]
pub struct PushResult {
    pub id: u64,
    pub hash: String,
}

/// Reference to a kv entry by either its numeric ID or its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdRef {
    Numeric(u64),
    Hash(String),
}

// ---------------------------------------------------------------------------
// KV Store
// ---------------------------------------------------------------------------

pub struct KvStore {
    pub schema: Schema,
    pub data: DataFile,
    pub data_path: PathBuf,
    pub schema_path: PathBuf,
}

impl KvStore {
    /// Load schema and data from the given paths. Creates data file with defaults
    /// if it doesn't exist.
    pub fn load(schema_path: &Path, data_path: &Path) -> Result<Self> {
        let schema_str = fs::read_to_string(schema_path)
            .with_context(|| format!("Failed to read schema: {}", schema_path.display()))?;
        let schema: Schema = toml::from_str(&schema_str)
            .with_context(|| format!("Failed to parse schema: {}", schema_path.display()))?;

        let mut data = if data_path.exists() {
            let data_str = fs::read_to_string(data_path)
                .with_context(|| format!("Failed to read data: {}", data_path.display()))?;
            serde_json::from_str(&data_str)
                .with_context(|| format!("Failed to parse data: {}", data_path.display()))?
        } else {
            DataFile::default()
        };

        // Back-fill hashes for entries loaded without one (pre-hash data).
        let mut needs_save = false;
        for (key, value) in &mut data.entries {
            match value {
                DataValue::History { entries, .. } => {
                    for e in entries.iter_mut() {
                        if e.hash.is_empty() {
                            e.hash = generate_entry_hash(key, &e.ts, e.id);
                            needs_save = true;
                        }
                    }
                }
                DataValue::List { items, .. } => {
                    for e in items.iter_mut() {
                        if e.hash.is_empty() {
                            e.hash = generate_entry_hash(key, &e.ts, e.id);
                            needs_save = true;
                        }
                    }
                }
                _ => {}
            }
        }

        let mut store = KvStore {
            schema,
            data,
            data_path: data_path.to_path_buf(),
            schema_path: schema_path.to_path_buf(),
        };

        if needs_save {
            store.save()?;
        }

        Ok(store)
    }

    /// Load from environment variables. Resolves {agent} placeholder.
    ///
    /// Resolution order for schema:
    /// 1. `MX_KV_SCHEMA` env var (with `{agent}` placeholder substitution)
    /// 2. `$MX_HOME/kv/schema/{agent}.toml` (new default)
    /// 3. Soft fallback: `~/.crewu/kv/{agent}.schema.toml` (legacy, with stderr note)
    ///
    /// Same shape for data via `MX_KV_DATA` and `~/.crewu/kv/{agent}.data.json`.
    ///
    /// TODO(kv-path-migration): remove the `~/.crewu/kv/` fallback after one
    /// release cycle.
    pub fn from_env() -> Result<Self> {
        let agent = std::env::var("MX_CURRENT_AGENT")
            .with_context(|| "MX_CURRENT_AGENT environment variable is required")?;

        let (schema_path, schema_warn) = Self::resolve_schema_path(&agent);
        let (data_path, data_warn) = Self::resolve_data_path(&agent);

        // Suppress the legacy warning when resolved files live under MX_HOME --
        // the user declared this directory as home, so files there with legacy
        // naming aren't really "legacy."
        let under_mx_home = schema_path.starts_with(crate::paths::mx_home())
            && data_path.starts_with(crate::paths::mx_home());
        if !under_mx_home
            && should_emit_legacy_kv_warning(schema_warn, data_warn, &LEGACY_KV_WARNING_EMITTED)
        {
            eprintln!("{}", LEGACY_KV_WARNING);
        }

        let mut store = Self::load(&schema_path, &data_path)?;

        // Populate _schema field from agent name if empty (SHOULD-FIX 5)
        if store.data.schema_id.is_empty() {
            store.data.schema_id = agent.clone();
        }

        Ok(store)
    }

    /// Resolve the schema path for an agent, delegating to the testable
    /// `resolve_kv_path_with` seam. Returns `(path, should_warn)` where
    /// `should_warn` is true only if the legacy `~/.crewu/kv/` location was
    /// used as a soft fallback.
    fn resolve_schema_path(agent: &str) -> (PathBuf, bool) {
        resolve_kv_path_with(
            std::env::var("MX_KV_SCHEMA").ok().as_deref(),
            agent,
            crate::paths::kv_schema_path(agent),
            crate::paths::legacy_crewu_kv_schema_path(agent),
        )
    }

    /// Resolve the data path for an agent, delegating to the testable
    /// `resolve_kv_path_with` seam. Returns `(path, should_warn)` where
    /// `should_warn` is true only if the legacy `~/.crewu/kv/` location was
    /// used as a soft fallback.
    fn resolve_data_path(agent: &str) -> (PathBuf, bool) {
        resolve_kv_path_with(
            std::env::var("MX_KV_DATA").ok().as_deref(),
            agent,
            crate::paths::kv_data_path(agent),
            crate::paths::legacy_crewu_kv_data_path(agent),
        )
    }

    /// Atomic write: serialize to tmp, fsync, rename.
    pub fn save(&mut self) -> Result<()> {
        self.data.updated = Utc::now().to_rfc3339();

        let json = serde_json::to_string_pretty(&self.data).context("Failed to serialize data")?;

        // Ensure parent directory exists
        if let Some(parent) = self.data_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let tmp_path = self
            .data_path
            .with_extension(format!("tmp.{}", std::process::id()));

        {
            let mut f = fs::File::create(&tmp_path)
                .with_context(|| format!("Failed to create temp file: {}", tmp_path.display()))?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
        }

        fs::rename(&tmp_path, &self.data_path).with_context(|| {
            format!(
                "Failed to rename {} -> {}",
                tmp_path.display(),
                self.data_path.display()
            )
        })?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Schema helpers
    // -----------------------------------------------------------------------

    fn key_def(&self, key: &str) -> Result<&KeyDef, KvError> {
        self.schema
            .keys
            .get(key)
            .ok_or_else(|| KvError::KeyNotFound(key.to_string()))
    }

    fn assert_type(&self, key: &str, expected: ValueType) -> Result<&KeyDef, KvError> {
        let def = self.key_def(key)?;
        if def.value_type != expected {
            return Err(KvError::TypeMismatch {
                key: key.to_string(),
                expected: expected.to_string(),
                got: def.value_type.to_string(),
            });
        }
        Ok(def)
    }

    /// Validate a key name for schema insertion.
    ///
    /// Accepts alphanumeric characters, underscores, and hyphens.
    /// Rejects names containing dots (TOML quoting issues), empty names,
    /// and names with other special characters.
    fn validate_key_name(key: &str) -> Result<(), KvError> {
        if key.is_empty() {
            return Err(KvError::Other(anyhow::anyhow!("key name cannot be empty")));
        }
        if key.len() > 128 {
            return Err(KvError::Other(anyhow::anyhow!(
                "key name too long ({} chars, max 128)",
                key.len()
            )));
        }
        if key.contains('.') {
            return Err(KvError::Other(anyhow::anyhow!(
                "key name '{}' cannot contain dots -- they require TOML quoting and create confusion",
                key
            )));
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(KvError::Other(anyhow::anyhow!(
                "key name '{}' contains invalid characters -- only alphanumeric, underscores, and hyphens are allowed",
                key
            )));
        }
        Ok(())
    }

    /// Add a new key to the schema file and reload the in-memory schema.
    ///
    /// Appends a `[keys.<name>]` block to the TOML file (preserving existing
    /// content exactly) and re-parses the file to update `self.schema`.
    ///
    /// Only `history` and `list` types are accepted -- those are the types
    /// that support `push`.
    pub fn add_key_to_schema(
        &mut self,
        key: &str,
        value_type: &str,
        max_entries: Option<usize>,
    ) -> Result<(), KvError> {
        // Validate key name
        Self::validate_key_name(key)?;

        // Double-check: don't overwrite existing keys
        if self.schema.keys.contains_key(key) {
            return Ok(());
        }

        // Build the TOML block to append
        let mut block = format!("\n[keys.{}]\ntype = \"{}\"\n", key, value_type);
        if let Some(max) = max_entries {
            block.push_str(&format!("max_entries = {}\n", max));
        }

        // Append to the schema file
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&self.schema_path)
            .map_err(|e| {
                KvError::Other(anyhow::anyhow!(
                    "failed to open schema file for append: {}: {}",
                    self.schema_path.display(),
                    e
                ))
            })?;
        f.write_all(block.as_bytes()).map_err(|e| {
            KvError::Other(anyhow::anyhow!(
                "failed to write to schema file: {}: {}",
                self.schema_path.display(),
                e
            ))
        })?;
        drop(f);

        // Re-read and re-parse the schema to update in-memory state
        let schema_str = fs::read_to_string(&self.schema_path).map_err(|e| {
            KvError::Other(anyhow::anyhow!(
                "failed to re-read schema file after append: {}: {}",
                self.schema_path.display(),
                e
            ))
        })?;
        let schema: Schema = toml::from_str(&schema_str).map_err(|e| {
            KvError::Other(anyhow::anyhow!(
                "failed to re-parse schema file after append: {}: {}",
                self.schema_path.display(),
                e
            ))
        })?;
        self.schema = schema;

        Ok(())
    }

    /// Get the default DataValue for a key based on its schema definition.
    fn default_value(def: &KeyDef) -> DataValue {
        match def.value_type {
            ValueType::Counter => {
                let default_val: i64 = def
                    .default
                    .as_ref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                DataValue::Counter { value: default_val }
            }
            ValueType::String => DataValue::String {
                value: def.default.clone().unwrap_or_default(),
            },
            ValueType::History => DataValue::History {
                entries: Vec::new(),
                memory: None,
            },
            ValueType::State => {
                let fields = def
                    .fields
                    .as_ref()
                    .map(|fs| fs.iter().map(|f| (f.clone(), String::new())).collect())
                    .unwrap_or_default();
                DataValue::State {
                    fields,
                    memory: None,
                }
            }
            ValueType::List => DataValue::List {
                items: Vec::new(),
                memory: None,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    /// Get the current value for a key.
    pub fn get(&self, key: &str) -> Result<&DataValue, KvError> {
        let def = self.key_def(key)?;
        match self.data.entries.get(key) {
            Some(v) => Ok(v),
            None => Err(KvError::KeyNotFound(format!(
                "{} (has no data yet, type: {})",
                key, def.value_type
            ))),
        }
    }

    /// Set a string or counter value, or set a field on a state type.
    pub fn set(&mut self, key: &str, value: &str, field: Option<&str>) -> Result<(), KvError> {
        let def = self.key_def(key)?.clone();

        match def.value_type {
            ValueType::String => {
                self.data.entries.insert(
                    key.to_string(),
                    DataValue::String {
                        value: value.to_string(),
                    },
                );
            }
            ValueType::Counter => {
                let v: i64 = value.parse().map_err(|_| {
                    KvError::Other(anyhow::anyhow!("Invalid counter value: {}", value))
                })?;
                let v = Self::clamp(v, def.min, def.max);
                self.data
                    .entries
                    .insert(key.to_string(), DataValue::Counter { value: v });
            }
            ValueType::State => {
                let field_name = field.ok_or_else(|| {
                    KvError::Other(anyhow::anyhow!(
                        "State type requires field name: mx kv set {} <field> <value>",
                        key
                    ))
                })?;

                // Validate field name against schema
                if let Some(ref schema_fields) = def.fields
                    && !schema_fields.contains(&field_name.to_string())
                {
                    return Err(KvError::Other(anyhow::anyhow!(
                        "Unknown field '{}' for key '{}'. Valid fields: {}",
                        field_name,
                        key,
                        schema_fields.join(", ")
                    )));
                }

                let entry = self
                    .data
                    .entries
                    .entry(key.to_string())
                    .or_insert_with(|| Self::default_value(&def));

                match entry {
                    DataValue::State { fields, .. } => {
                        fields.insert(field_name.to_string(), value.to_string());
                    }
                    _ => {
                        return Err(KvError::Other(anyhow::anyhow!(
                            "Data corruption: key '{}' has wrong runtime type",
                            key
                        )));
                    }
                }
            }
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "string, counter, or state".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Increment a counter. Clamps to min/max, never errors on bounds.
    pub fn inc(&mut self, key: &str, by: i64) -> Result<i64, KvError> {
        let def = self.assert_type(key, ValueType::Counter)?.clone();

        let entry = self
            .data
            .entries
            .entry(key.to_string())
            .or_insert_with(|| Self::default_value(&def));

        match entry {
            DataValue::Counter { value } => {
                *value = Self::clamp(value.saturating_add(by), def.min, def.max);
                Ok(*value)
            }
            _ => Err(KvError::Other(anyhow::anyhow!(
                "Data corruption: key '{}' has wrong runtime type",
                key
            ))),
        }
    }

    /// Decrement a counter. Clamps to min/max, never errors on bounds.
    pub fn dec(&mut self, key: &str, by: i64) -> Result<i64, KvError> {
        let def = self.assert_type(key, ValueType::Counter)?.clone();

        let entry = self
            .data
            .entries
            .entry(key.to_string())
            .or_insert_with(|| Self::default_value(&def));

        match entry {
            DataValue::Counter { value } => {
                *value = Self::clamp(value.saturating_sub(by), def.min, def.max);
                Ok(*value)
            }
            _ => Err(KvError::Other(anyhow::anyhow!(
                "Data corruption: key '{}' has wrong runtime type",
                key
            ))),
        }
    }

    /// Push a value onto a history (with auto-timestamp) or list.
    pub fn push(
        &mut self,
        key: &str,
        value: &str,
        data: Option<serde_json::Value>,
        memory: Option<String>,
    ) -> Result<PushResult, KvError> {
        self.push_with_ts(key, value, Utc::now(), data, memory)
    }

    /// Push with an explicit timestamp (used by tests).
    pub fn push_with_ts(
        &mut self,
        key: &str,
        value: &str,
        ts: DateTime<Utc>,
        data: Option<serde_json::Value>,
        memory: Option<String>,
    ) -> Result<PushResult, KvError> {
        let def = self.key_def(key)?.clone();
        let ts_str = ts.to_rfc3339();

        match def.value_type {
            ValueType::History => {
                let entry = self
                    .data
                    .entries
                    .entry(key.to_string())
                    .or_insert_with(|| Self::default_value(&def));

                match entry {
                    DataValue::History { entries, .. } => {
                        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                        let hash = generate_entry_hash(key, &ts_str, next_id);
                        entries.insert(
                            0,
                            HistoryEntry {
                                id: next_id,
                                hash: hash.clone(),
                                value: value.to_string(),
                                ts: ts_str,
                                data,
                                memory,
                            },
                        );
                        // Drop oldest at max_entries
                        if let Some(max) = def.max_entries {
                            entries.truncate(max);
                        }
                        Ok(PushResult { id: next_id, hash })
                    }
                    _ => Err(KvError::Other(anyhow::anyhow!(
                        "Data corruption: key '{}' has wrong runtime type",
                        key
                    ))),
                }
            }
            ValueType::List => {
                let entry = self
                    .data
                    .entries
                    .entry(key.to_string())
                    .or_insert_with(|| Self::default_value(&def));

                match entry {
                    DataValue::List { items, .. } => {
                        let next_id = items.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                        let hash = generate_entry_hash(key, &ts_str, next_id);
                        items.push(ListEntry {
                            id: next_id,
                            hash: hash.clone(),
                            value: value.to_string(),
                            ts: ts_str,
                            data,
                            memory,
                        });
                        // Drop oldest at max_entries — single drain instead of O(n^2) remove loop
                        if let Some(max) = def.max_entries
                            && items.len() > max
                        {
                            items.drain(0..items.len() - max);
                        }
                        Ok(PushResult { id: next_id, hash })
                    }
                    _ => Err(KvError::Other(anyhow::anyhow!(
                        "Data corruption: key '{}' has wrong runtime type",
                        key
                    ))),
                }
            }
            _ => Err(KvError::TypeMismatch {
                key: key.to_string(),
                expected: "history or list".to_string(),
                got: def.value_type.to_string(),
            }),
        }
    }

    /// Pop the last item from a list. History is append-only.
    pub fn pop(&mut self, key: &str) -> Result<Option<ListEntry>, KvError> {
        self.assert_type(key, ValueType::List)?;

        match self.data.entries.get_mut(key) {
            Some(DataValue::List { items, .. }) => Ok(items.pop()),
            Some(_) => Err(KvError::Other(anyhow::anyhow!(
                "Data corruption: key '{}' has wrong runtime type",
                key
            ))),
            None => Ok(None),
        }
    }

    /// Get the last N entries from a history or list as `SearchHit`s.
    ///
    /// When `range` is provided, entries are filtered by timestamp first, then
    /// the `count` limit is applied to the filtered set.
    /// When `where_clauses` is non-empty, entries must match all clauses.
    pub fn last(
        &self,
        key: &str,
        count: usize,
        range: Option<&TimeRange>,
        where_clauses: &[(String, String)],
    ) -> Result<Vec<SearchHit>, KvError> {
        let def = self.key_def(key)?;

        match def.value_type {
            ValueType::History => match self.data.entries.get(key) {
                Some(DataValue::History { entries, .. }) => {
                    // History stores newest-first; reverse so filtered vec is
                    // chronological (oldest-first), matching the List branch.
                    let filtered: Vec<_> = entries
                        .iter()
                        .rev()
                        .filter(|e| {
                            range.is_none_or(|r| ts_in_range(&e.ts, r))
                                && where_matches(&e.data, where_clauses)
                        })
                        .collect();
                    let start = filtered.len().saturating_sub(count);
                    Ok(filtered[start..]
                        .iter()
                        .map(|e| SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        })
                        .collect())
                }
                _ => Ok(vec![]),
            },
            ValueType::List => match self.data.entries.get(key) {
                Some(DataValue::List { items, .. }) => {
                    let filtered: Vec<_> = items
                        .iter()
                        .filter(|e| {
                            range.is_none_or(|r| ts_in_range(&e.ts, r))
                                && where_matches(&e.data, where_clauses)
                        })
                        .collect();
                    let start = filtered.len().saturating_sub(count);
                    Ok(filtered[start..]
                        .iter()
                        .map(|e| SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        })
                        .collect())
                }
                _ => Ok(vec![]),
            },
            _ => Err(KvError::TypeMismatch {
                key: key.to_string(),
                expected: "history or list".to_string(),
                got: def.value_type.to_string(),
            }),
        }
    }

    /// Get random entries from a history or list as `SearchHit`s.
    ///
    /// When `range` is provided, entries are filtered by timestamp first, then
    /// `count` random items are sampled from the filtered set. If fewer entries
    /// are available than requested, all matching entries are returned and a note
    /// is printed to stderr.
    /// When `where_clauses` is non-empty, entries must match all clauses.
    pub fn random(
        &self,
        key: &str,
        count: usize,
        range: Option<&TimeRange>,
        where_clauses: &[(String, String)],
    ) -> Result<Vec<SearchHit>, KvError> {
        use rand::seq::IndexedRandom;

        let def = self.key_def(key)?;

        /// Shared sampling helper. Takes a filtered vec of `SearchHit` references
        /// and returns `count` randomly chosen hits (cloned).
        fn sample_hits(filtered: &[SearchHit], n: usize) -> Vec<SearchHit> {
            if filtered.is_empty() {
                return vec![];
            }
            let take = n.min(filtered.len());
            let mut rng = rand::rng();
            filtered.choose_multiple(&mut rng, take).cloned().collect()
        }

        match def.value_type {
            ValueType::History => match self.data.entries.get(key) {
                Some(DataValue::History { entries, .. }) => {
                    let filtered: Vec<SearchHit> = entries
                        .iter()
                        .filter(|e| {
                            range.is_none_or(|r| ts_in_range(&e.ts, r))
                                && where_matches(&e.data, where_clauses)
                        })
                        .map(|e| SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        })
                        .collect();
                    let available = filtered.len();
                    if available == 0 && !entries.is_empty() {
                        eprintln!("note: no entries match the time range");
                    } else if available > 0 && available < count {
                        eprintln!(
                            "note: only {} entries available (requested {})",
                            available, count
                        );
                    }
                    Ok(sample_hits(&filtered, count))
                }
                _ => Ok(vec![]),
            },
            ValueType::List => match self.data.entries.get(key) {
                Some(DataValue::List { items, .. }) => {
                    let filtered: Vec<SearchHit> = items
                        .iter()
                        .filter(|e| {
                            range.is_none_or(|r| ts_in_range(&e.ts, r))
                                && where_matches(&e.data, where_clauses)
                        })
                        .map(|e| SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        })
                        .collect();
                    let available = filtered.len();
                    if available == 0 && !items.is_empty() {
                        eprintln!("note: no entries match the time range");
                    } else if available > 0 && available < count {
                        eprintln!(
                            "note: only {} entries available (requested {})",
                            available, count
                        );
                    }
                    Ok(sample_hits(&filtered, count))
                }
                _ => Ok(vec![]),
            },
            _ => Err(KvError::TypeMismatch {
                key: key.to_string(),
                expected: "history or list".to_string(),
                got: def.value_type.to_string(),
            }),
        }
    }

    /// Get history entries since a given time reference.
    pub fn since(&self, key: &str, timeref: &str) -> Result<Vec<&HistoryEntry>, KvError> {
        self.assert_type(key, ValueType::History)?;

        let cutoff = parse_timeref(timeref).map_err(KvError::Other)?;

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => Ok(entries
                .iter()
                .filter(|e| {
                    DateTime::parse_from_rfc3339(&e.ts)
                        .map(|t| t >= cutoff)
                        .unwrap_or(false)
                })
                .collect()),
            _ => Ok(vec![]),
        }
    }

    /// Reset a key to its schema default.
    pub fn reset(&mut self, key: &str) -> Result<(), KvError> {
        let def = self.key_def(key)?.clone();
        self.data
            .entries
            .insert(key.to_string(), Self::default_value(&def));
        Ok(())
    }

    /// Remove entries from a list or history by value substring or by ID ref.
    ///
    /// - `by_id`: if Some, remove the entry matching that `IdRef` (ignores `value` and `all`).
    /// - `value`: substring match (case-insensitive).
    /// - `all`: if true, remove all matches; otherwise remove only the first match.
    pub fn remove(
        &mut self,
        key: &str,
        value: Option<&str>,
        by_id: Option<&IdRef>,
        all: bool,
    ) -> Result<RemoveResult, KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        let mut removed = Vec::new();

        match self.data.entries.get_mut(key) {
            Some(DataValue::History { entries, .. }) => {
                if let Some(id_ref) = by_id {
                    let pos = match id_ref {
                        IdRef::Numeric(id) => entries.iter().position(|e| e.id == *id),
                        IdRef::Hash(h) => {
                            let matches: Vec<usize> = entries
                                .iter()
                                .enumerate()
                                .filter(|(_, e)| e.hash.starts_with(h.as_str()))
                                .map(|(i, _)| i)
                                .collect();
                            match matches.len() {
                                0 => None,
                                1 => Some(matches[0]),
                                n => {
                                    return Err(KvError::AmbiguousHash {
                                        prefix: h.clone(),
                                        count: n,
                                    });
                                }
                            }
                        }
                    };
                    if let Some(pos) = pos {
                        removed.push(entries.remove(pos).value);
                    }
                } else if let Some(query) = value {
                    let query_lower = query.to_lowercase();
                    let mut found_first = false;
                    entries.retain(|e| {
                        if e.value.to_lowercase().contains(&query_lower) && (all || !found_first) {
                            found_first = true;
                            removed.push(e.value.clone());
                            return false;
                        }
                        true
                    });
                }
            }
            Some(DataValue::List { items, .. }) => {
                if let Some(id_ref) = by_id {
                    let pos = match id_ref {
                        IdRef::Numeric(id) => items.iter().position(|e| e.id == *id),
                        IdRef::Hash(h) => {
                            let matches: Vec<usize> = items
                                .iter()
                                .enumerate()
                                .filter(|(_, e)| e.hash.starts_with(h.as_str()))
                                .map(|(i, _)| i)
                                .collect();
                            match matches.len() {
                                0 => None,
                                1 => Some(matches[0]),
                                n => {
                                    return Err(KvError::AmbiguousHash {
                                        prefix: h.clone(),
                                        count: n,
                                    });
                                }
                            }
                        }
                    };
                    if let Some(pos) = pos {
                        removed.push(items.remove(pos).value);
                    }
                } else if let Some(query) = value {
                    let query_lower = query.to_lowercase();
                    let mut found_first = false;
                    items.retain(|e| {
                        if e.value.to_lowercase().contains(&query_lower) && (all || !found_first) {
                            found_first = true;
                            removed.push(e.value.clone());
                            return false;
                        }
                        true
                    });
                }
            }
            _ => {} // no data yet, nothing to remove
        }

        Ok(RemoveResult { removed })
    }

    /// Search entries in a list or history by case-insensitive substring and/or
    /// structured data filters.
    ///
    /// When `query` is `Some`, entries must contain the substring.
    /// When `where_clauses` is non-empty, entries must match all clauses.
    /// When `range` is provided, only entries within the time range are searched.
    pub fn search(
        &self,
        key: &str,
        query: Option<&str>,
        range: Option<&TimeRange>,
        where_clauses: &[(String, String)],
    ) -> Result<Vec<SearchHit>, KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        let query_lower = query.map(|q| q.to_lowercase());
        let mut hits = Vec::new();

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => {
                for e in entries {
                    if !range.is_none_or(|r| ts_in_range(&e.ts, r)) {
                        continue;
                    }
                    if let Some(ref q) = query_lower
                        && !e.value.to_lowercase().contains(q)
                    {
                        continue;
                    }
                    if !where_matches(&e.data, where_clauses) {
                        continue;
                    }
                    hits.push(SearchHit {
                        id: e.id,
                        hash: e.hash.clone(),
                        value: e.value.clone(),
                        ts: e.ts.clone(),
                        data: e.data.clone(),
                        memory: e.memory.clone(),
                    });
                }
            }
            Some(DataValue::List { items, .. }) => {
                for e in items {
                    if !range.is_none_or(|r| ts_in_range(&e.ts, r)) {
                        continue;
                    }
                    if let Some(ref q) = query_lower
                        && !e.value.to_lowercase().contains(q)
                    {
                        continue;
                    }
                    if !where_matches(&e.data, where_clauses) {
                        continue;
                    }
                    hits.push(SearchHit {
                        id: e.id,
                        hash: e.hash.clone(),
                        value: e.value.clone(),
                        ts: e.ts.clone(),
                        data: e.data.clone(),
                        memory: e.memory.clone(),
                    });
                }
            }
            _ => {}
        }

        Ok(hits)
    }

    /// Look up entries by ID (numeric or hash) in a history or list.
    ///
    /// Returns matching entries as `SearchHit`s (same struct used by `search`).
    /// Hash matching is prefix-based: `"A3f"` matches an entry with hash `"A3fBx2"`.
    /// Only works on History and List types; returns TypeMismatch for others.
    pub fn get_entries_by_id(&self, key: &str, ids: &[IdRef]) -> Result<Vec<SearchHit>, KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        let numeric_ids: HashSet<u64> = ids
            .iter()
            .filter_map(|r| match r {
                IdRef::Numeric(n) => Some(*n),
                _ => None,
            })
            .collect();
        let hash_prefixes: Vec<&str> = ids
            .iter()
            .filter_map(|r| match r {
                IdRef::Hash(h) => Some(h.as_str()),
                _ => None,
            })
            .collect();

        let matches_entry = |id: u64, hash: &str| -> bool {
            if numeric_ids.contains(&id) {
                return true;
            }
            hash_prefixes.iter().any(|prefix| hash.starts_with(prefix))
        };

        let mut hits = Vec::new();

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => {
                for e in entries {
                    if matches_entry(e.id, &e.hash) {
                        hits.push(SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        });
                    }
                }
            }
            Some(DataValue::List { items, .. }) => {
                for e in items {
                    if matches_entry(e.id, &e.hash) {
                        hits.push(SearchHit {
                            id: e.id,
                            hash: e.hash.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        });
                    }
                }
            }
            _ => {} // Key defined in schema but no entries pushed yet
        }

        Ok(hits)
    }

    /// Count entries in a list or history, optionally filtered by substring,
    /// time range, and/or structured data filters.
    ///
    /// When `range` is provided, only entries within the time range are counted.
    /// When `where_clauses` is non-empty, entries must match all clauses.
    /// The `total` field in the result reflects entries that passed the time filter
    /// (when a value filter or where filter is active).
    pub fn count(
        &self,
        key: &str,
        value: Option<&str>,
        range: Option<&TimeRange>,
        where_clauses: &[(String, String)],
    ) -> Result<CountResult, KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        let query_lower = value.map(|v| v.to_lowercase());
        let filtering = query_lower.is_some() || !where_clauses.is_empty();
        let mut matched = 0usize;
        let mut entry_total = 0usize;
        let mut latest_ts: Option<String> = None;

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => {
                for e in entries {
                    if !range.is_none_or(|r| ts_in_range(&e.ts, r)) {
                        continue;
                    }
                    entry_total += 1;
                    let text_match = match &query_lower {
                        Some(q) => e.value.to_lowercase().contains(q),
                        None => true,
                    };
                    let is_match = text_match && where_matches(&e.data, where_clauses);
                    if is_match {
                        matched += 1;
                        if latest_ts.is_none() || e.ts > *latest_ts.as_ref().unwrap() {
                            latest_ts = Some(e.ts.clone());
                        }
                    }
                }
            }
            Some(DataValue::List { items, .. }) => {
                for e in items {
                    if !range.is_none_or(|r| ts_in_range(&e.ts, r)) {
                        continue;
                    }
                    entry_total += 1;
                    let text_match = match &query_lower {
                        Some(q) => e.value.to_lowercase().contains(q),
                        None => true,
                    };
                    let is_match = text_match && where_matches(&e.data, where_clauses);
                    if is_match {
                        matched += 1;
                        if !e.ts.is_empty()
                            && (latest_ts.is_none() || e.ts > *latest_ts.as_ref().unwrap())
                        {
                            latest_ts = Some(e.ts.clone());
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(CountResult {
            matched,
            total: if filtering { Some(entry_total) } else { None },
            latest_ts,
        })
    }

    /// List all keys with their types.
    pub fn keys(&self) -> Vec<(&str, ValueType)> {
        self.schema
            .keys
            .iter()
            .map(|(k, v)| (k.as_str(), v.value_type))
            .collect()
    }

    /// Dump all state as JSON.
    pub fn dump_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.data).context("Failed to serialize data")
    }

    /// Dump all state in compact format for wake integration.
    pub fn dump_compact(&self) -> String {
        let mut parts = Vec::new();

        for (key, def) in &self.schema.keys {
            let part = match self.data.entries.get(key) {
                Some(val) => format_compact(key, val, def),
                None => format_compact(key, &Self::default_value(def), def),
            };
            parts.push(part);
        }

        parts.join(" ")
    }

    // -----------------------------------------------------------------------
    // Memory pointer operations
    // -----------------------------------------------------------------------

    /// Set the memory pointer (kn- reference) on a history, list, or state key.
    /// Pass `None` to clear the pointer.
    pub fn set_memory(&mut self, key: &str, memory: Option<String>) -> Result<(), KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List | ValueType::State => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history, list, or state".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        let def = def.clone();
        let entry = self
            .data
            .entries
            .entry(key.to_string())
            .or_insert_with(|| Self::default_value(&def));

        // Normalize empty string to None (clearing the link)
        let memory = memory.filter(|s| !s.is_empty());

        match entry {
            DataValue::History { memory: mem, .. } => *mem = memory,
            DataValue::List { memory: mem, .. } => *mem = memory,
            DataValue::State { memory: mem, .. } => *mem = memory,
            _ => unreachable!(),
        }

        Ok(())
    }

    /// Get the memory pointer for a key, if set.
    pub fn get_memory(&self, key: &str) -> Result<Option<&str>, KvError> {
        self.key_def(key)?; // validate key exists in schema

        match self.data.entries.get(key) {
            Some(DataValue::History { memory, .. }) => Ok(memory.as_deref()),
            Some(DataValue::List { memory, .. }) => Ok(memory.as_deref()),
            Some(DataValue::State { memory, .. }) => Ok(memory.as_deref()),
            Some(DataValue::Counter { .. } | DataValue::String { .. }) => {
                Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history, list, or state".to_string(),
                    got: "counter or string".to_string(),
                })
            }
            None => Ok(None),
        }
    }

    /// Set or clear the memory pointer on a specific entry within a history or list.
    ///
    /// Pass `None` (or `Some("")`) to clear the pointer.
    /// Returns `KvError::EntryNotFound` if the entry is not found.
    pub fn set_entry_memory(
        &mut self,
        key: &str,
        id: &IdRef,
        memory: Option<String>,
    ) -> Result<(), KvError> {
        let def = self.key_def(key)?;
        match def.value_type {
            ValueType::History | ValueType::List => {}
            _ => {
                return Err(KvError::TypeMismatch {
                    key: key.to_string(),
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        // Normalize empty string to None (clearing the link)
        let memory = memory.filter(|s| !s.is_empty());

        match self.data.entries.get_mut(key) {
            Some(DataValue::History { entries, .. }) => {
                let entry = match id {
                    IdRef::Numeric(n) => entries.iter_mut().find(|e| e.id == *n),
                    IdRef::Hash(h) => {
                        let matches: Vec<_> = entries
                            .iter_mut()
                            .filter(|e| e.hash.starts_with(h.as_str()))
                            .collect();
                        match matches.len() {
                            0 => None,
                            1 => {
                                // Re-find to satisfy borrow checker (collected vec consumes &mut)
                                entries.iter_mut().find(|e| e.hash.starts_with(h.as_str()))
                            }
                            n => {
                                return Err(KvError::AmbiguousHash {
                                    prefix: h.clone(),
                                    count: n,
                                });
                            }
                        }
                    }
                };
                match entry {
                    Some(e) => {
                        e.memory = memory;
                        Ok(())
                    }
                    None => Err(KvError::EntryNotFound {
                        key: key.to_string(),
                        id: format!("{:?}", id),
                    }),
                }
            }
            Some(DataValue::List { items, .. }) => {
                let entry = match id {
                    IdRef::Numeric(n) => items.iter_mut().find(|e| e.id == *n),
                    IdRef::Hash(h) => {
                        let matches: Vec<_> = items
                            .iter_mut()
                            .filter(|e| e.hash.starts_with(h.as_str()))
                            .collect();
                        match matches.len() {
                            0 => None,
                            1 => items.iter_mut().find(|e| e.hash.starts_with(h.as_str())),
                            n => {
                                return Err(KvError::AmbiguousHash {
                                    prefix: h.clone(),
                                    count: n,
                                });
                            }
                        }
                    }
                };
                match entry {
                    Some(e) => {
                        e.memory = memory;
                        Ok(())
                    }
                    None => Err(KvError::EntryNotFound {
                        key: key.to_string(),
                        id: format!("{:?}", id),
                    }),
                }
            }
            _ => Err(KvError::EntryNotFound {
                key: key.to_string(),
                id: format!("{:?}", id),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn clamp(value: i64, min: Option<i64>, max: Option<i64>) -> i64 {
        let mut v = value;
        if let Some(lo) = min {
            v = v.max(lo);
        }
        if let Some(hi) = max {
            v = v.min(hi);
        }
        v
    }
}

// ---------------------------------------------------------------------------
// Structured data filter
// ---------------------------------------------------------------------------

/// Check whether an entry's structured data matches all `(key, value)` clauses.
///
/// Semantics:
/// - If `data` is `None` and clauses is non-empty, return `false`.
/// - For each `(key, value)` clause, `data[key]` must equal the string value
///   OR `data[key]` must be an array that contains the string value.
/// - ALL clauses must match (AND logic).
pub fn where_matches(data: &Option<serde_json::Value>, clauses: &[(String, String)]) -> bool {
    if clauses.is_empty() {
        return true;
    }

    let obj = match data {
        Some(serde_json::Value::Object(map)) => map,
        _ => return false,
    };

    clauses.iter().all(|(key, value)| match obj.get(key) {
        Some(serde_json::Value::String(s)) => s == value,
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .any(|v| matches!(v, serde_json::Value::String(s) if s == value)),
        Some(serde_json::Value::Number(n)) => n.to_string() == *value,
        Some(serde_json::Value::Bool(b)) => b.to_string() == *value,
        _ => false,
    })
}

/// Format the data suffix for display output.
///
/// When data is `Some`, returns ` {compact_json}`. When `None`, returns empty string.
pub fn format_data_suffix(data: &Option<serde_json::Value>) -> String {
    match data {
        Some(v) => format!(" {}", serde_json::to_string(v).unwrap_or_default()),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Compact format
// ---------------------------------------------------------------------------

fn format_compact(key: &str, value: &DataValue, def: &KeyDef) -> String {
    match value {
        DataValue::Counter { value } => format!("{}={}", key, value),
        DataValue::String { value } => format!("{}={}", key, value),
        DataValue::History {
            entries, memory, ..
        } => {
            let items: Vec<String> = entries
                .iter()
                .map(|e| {
                    let time_part = DateTime::parse_from_rfc3339(&e.ts)
                        .map(|t| t.format("%H:%M").to_string())
                        .unwrap_or_else(|_| "??:??".to_string());
                    format!("{}@{}", e.value, time_part)
                })
                .collect();
            let base = format!("{}=[{}]", key, items.join(","));
            match memory {
                Some(m) if !m.is_empty() => format!("{}({})", base, m),
                _ => base,
            }
        }
        DataValue::State { fields, memory, .. } => {
            // Values only, ordered by schema field order
            let values: Vec<String> = def
                .fields
                .as_ref()
                .map(|schema_fields| {
                    schema_fields
                        .iter()
                        .map(|f| fields.get(f).cloned().unwrap_or_default())
                        .collect()
                })
                .unwrap_or_else(|| fields.values().cloned().collect());
            let base = format!("{}={{{}}}", key, values.join(","));
            match memory {
                Some(m) if !m.is_empty() => format!("{}({})", base, m),
                _ => base,
            }
        }
        DataValue::List { items, memory, .. } => {
            let formatted: Vec<String> = items
                .iter()
                .map(|e| {
                    if e.ts.is_empty() {
                        e.value.clone()
                    } else {
                        let time_part = DateTime::parse_from_rfc3339(&e.ts)
                            .map(|t| t.format("%H:%M").to_string())
                            .unwrap_or_else(|_| "??:??".to_string());
                        format!("{}@{}", e.value, time_part)
                    }
                })
                .collect();
            let base = format!("{}=[{}]", key, formatted.join(","));
            match memory {
                Some(m) if !m.is_empty() => format!("{}({})", base, m),
                _ => base,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Relative time parser
// ---------------------------------------------------------------------------

/// Parse a time reference: either ISO-8601 or relative (1h, 7d, 2w, 30m).
pub fn parse_timeref(timeref: &str) -> Result<DateTime<Utc>> {
    // Try ISO-8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(timeref) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try relative time
    parse_relative_time(timeref)
}

/// Parse a relative time string like "1h", "7d", "2w", "30m".
pub fn parse_relative_time(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        bail!("Empty time reference");
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str
        .parse()
        .with_context(|| format!("Invalid number in time reference: '{}'", s))?;

    let duration = match unit {
        "m" => chrono::Duration::minutes(num),
        "h" => chrono::Duration::hours(num),
        "d" => chrono::Duration::days(num),
        "w" => chrono::Duration::weeks(num),
        _ => bail!(
            "Unknown time unit '{}' in '{}'. Use m (minutes), h (hours), d (days), w (weeks)",
            unit,
            s
        ),
    };

    Ok(Utc::now() - duration)
}

// ---------------------------------------------------------------------------
// Time-range queries
// ---------------------------------------------------------------------------

/// Half-open time range `[from, to)` in UTC.
#[derive(Debug, Clone)]
pub struct TimeRange {
    /// Inclusive lower bound.
    pub from: DateTime<Utc>,
    /// Exclusive upper bound.
    pub to: DateTime<Utc>,
}

/// Parse `YYYY-MM-DD` into a day range `[start_of_day, start_of_next_day)`.
pub fn parse_day(s: &str) -> Result<TimeRange> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .with_context(|| format!("Invalid day format '{}', expected YYYY-MM-DD", s))?;
    let from = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let to = date
        .succ_opt()
        .with_context(|| format!("Day overflow for '{}'", s))?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    Ok(TimeRange { from, to })
}

/// Parse `YYYY-MM` into a month range `[first_of_month, first_of_next_month)`.
pub fn parse_month(s: &str) -> Result<TimeRange> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        bail!("Invalid month format '{}', expected YYYY-MM", s);
    }
    let year: i32 = parts[0]
        .parse()
        .with_context(|| format!("Invalid year in month '{}'", s))?;
    let month: u32 = parts[1]
        .parse()
        .with_context(|| format!("Invalid month number in '{}'", s))?;
    if !(1..=12).contains(&month) {
        bail!("Month out of range in '{}'", s);
    }

    let from_date = NaiveDate::from_ymd_opt(year, month, 1)
        .with_context(|| format!("Invalid month '{}'", s))?;
    let from = from_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

    // First of next month (handle December -> January rollover)
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let to_date = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .with_context(|| format!("Month overflow for '{}'", s))?;
    let to = to_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

    Ok(TimeRange { from, to })
}

/// Parse `YYYY-Www` (ISO week) into a range `[Monday, next_Monday)`.
pub fn parse_week(s: &str) -> Result<TimeRange> {
    // Expect format like "2026-W17"
    let parts: Vec<&str> = s.split("-W").collect();
    if parts.len() != 2 {
        bail!(
            "Invalid week format '{}', expected YYYY-Www (e.g. 2026-W17)",
            s
        );
    }
    let year: i32 = parts[0]
        .parse()
        .with_context(|| format!("Invalid year in week '{}'", s))?;
    let week: u32 = parts[1]
        .parse()
        .with_context(|| format!("Invalid week number in '{}'", s))?;
    if week == 0 || week > 53 {
        bail!("Week number out of range in '{}' (must be 1-53)", s);
    }

    let from_date = NaiveDate::from_isoywd_opt(year, week, chrono::Weekday::Mon)
        .with_context(|| format!("Invalid ISO week '{}'", s))?;
    let from = from_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

    let to_date = from_date + chrono::Duration::days(7);
    let to = to_date.and_hms_opt(0, 0, 0).unwrap().and_utc();

    Ok(TimeRange { from, to })
}

/// Parse an explicit `--from`/`--to` date range.
///
/// - Both provided: `[from, to+1day)` (inclusive of the to-date).
/// - `from` only: `[from, now)`.
/// - `to` only: `[epoch, to+1day)`.
pub fn parse_date_range(from: Option<&str>, to: Option<&str>) -> Result<TimeRange> {
    let range_from = match from {
        Some(s) => {
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("Invalid --from date '{}', expected YYYY-MM-DD", s))?;
            date.and_hms_opt(0, 0, 0).unwrap().and_utc()
        }
        None => {
            // Beginning of time (Unix epoch)
            DateTime::UNIX_EPOCH
        }
    };

    let range_to = match to {
        Some(s) => {
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("Invalid --to date '{}', expected YYYY-MM-DD", s))?;
            // Inclusive: end of that day = start of next day
            let next = date
                .succ_opt()
                .with_context(|| format!("Date overflow for --to '{}'", s))?;
            next.and_hms_opt(0, 0, 0).unwrap().and_utc()
        }
        None => Utc::now(),
    };

    if range_from >= range_to {
        bail!(
            "--from ({}) must be before --to ({})",
            range_from.format("%Y-%m-%d"),
            range_to.format("%Y-%m-%d")
        );
    }

    Ok(TimeRange {
        from: range_from,
        to: range_to,
    })
}

/// Top-level resolver: convert CLI `TimeRangeArgs` into an optional `TimeRange`.
///
/// Returns `Ok(None)` when no time-range flags were provided.
pub fn resolve_time_range(args: &TimeRangeArgs) -> Result<Option<TimeRange>> {
    if let Some(ref day) = args.day {
        return parse_day(day).map(Some);
    }
    if let Some(ref month) = args.month {
        return parse_month(month).map(Some);
    }
    if let Some(ref week) = args.week {
        return parse_week(week).map(Some);
    }
    if let Some(ref since) = args.since {
        let from = parse_timeref(since)?;
        let to = Utc::now();
        return Ok(Some(TimeRange { from, to }));
    }
    if args.range_from.is_some() || args.range_to.is_some() {
        return parse_date_range(args.range_from.as_deref(), args.range_to.as_deref()).map(Some);
    }
    Ok(None)
}

/// Check whether a timestamp string falls within a `TimeRange`.
///
/// Unparseable or empty timestamps never match.
pub fn ts_in_range(ts: &str, range: &TimeRange) -> bool {
    DateTime::parse_from_rfc3339(ts)
        .map(|t| {
            let t = t.with_timezone(&Utc);
            t >= range.from && t < range.to
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Display helpers (for CLI output)
// ---------------------------------------------------------------------------

/// Format a single entry line with numeric ID, hash, value, optional timestamp, and data suffix.
pub fn format_entry_line(
    id: u64,
    hash: &str,
    value: &str,
    ts: &str,
    data: &Option<serde_json::Value>,
) -> String {
    let data_suffix = format_data_suffix(data);
    if ts.is_empty() {
        format!("{} [kv-{}]: {}{}", id, hash, value, data_suffix)
    } else {
        format!("{} [kv-{}]: {} ({}){}", id, hash, value, ts, data_suffix)
    }
}

/// Format a DataValue for human-readable CLI output.
pub fn format_value(value: &DataValue) -> String {
    match value {
        DataValue::Counter { value } => value.to_string(),
        DataValue::String { value } => value.clone(),
        DataValue::History { entries, .. } => entries
            .iter()
            .map(|e| format_entry_line(e.id, &e.hash, &e.value, &e.ts, &e.data))
            .collect::<Vec<_>>()
            .join("\n"),
        DataValue::State { fields, .. } => {
            serde_json::to_string_pretty(fields).unwrap_or_else(|_| "{}".to_string())
        }
        DataValue::List { items, .. } => items
            .iter()
            .map(|e| format_entry_line(e.id, &e.hash, &e.value, &e.ts, &e.data))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use tempfile::TempDir;

    fn test_schema() -> &'static str {
        r#"
[keys.warmth]
type = "counter"
min = 0
default = "0"

[keys.capped]
type = "counter"
min = 0
max = 100
default = "50"

[keys.flavor_history]
type = "history"
max_entries = 3

[keys.tensor]
type = "state"
fields = ["temperature", "entropy", "agency"]

[keys.current_mood]
type = "string"
default = "neutral"

[keys.tags]
type = "list"
max_entries = 5
"#
    }

    fn setup_store(schema_toml: &str) -> (KvStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(schema_toml.as_bytes()).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();
        (store, dir)
    }

    // -- Schema parsing --

    #[test]
    fn parse_schema_all_types() {
        let (store, _dir) = setup_store(test_schema());
        assert_eq!(store.schema.keys.len(), 6);
        assert_eq!(store.schema.keys["warmth"].value_type, ValueType::Counter);
        assert_eq!(
            store.schema.keys["flavor_history"].value_type,
            ValueType::History
        );
        assert_eq!(store.schema.keys["tensor"].value_type, ValueType::State);
        assert_eq!(
            store.schema.keys["current_mood"].value_type,
            ValueType::String
        );
        assert_eq!(store.schema.keys["tags"].value_type, ValueType::List);
    }

    #[test]
    fn parse_schema_counter_constraints() {
        let (store, _dir) = setup_store(test_schema());
        let warmth = &store.schema.keys["warmth"];
        assert_eq!(warmth.min, Some(0));
        assert_eq!(warmth.max, None);

        let capped = &store.schema.keys["capped"];
        assert_eq!(capped.min, Some(0));
        assert_eq!(capped.max, Some(100));
        assert_eq!(capped.default.as_deref(), Some("50"));
    }

    #[test]
    fn parse_schema_state_fields() {
        let (store, _dir) = setup_store(test_schema());
        let tensor = &store.schema.keys["tensor"];
        assert_eq!(
            tensor.fields.as_ref().unwrap(),
            &["temperature", "entropy", "agency"]
        );
    }

    // -- Counter operations --

    #[test]
    fn counter_inc_dec() {
        let (mut store, _dir) = setup_store(test_schema());
        assert_eq!(store.inc("warmth", 1).unwrap(), 1);
        assert_eq!(store.inc("warmth", 5).unwrap(), 6);
        assert_eq!(store.dec("warmth", 2).unwrap(), 4);
    }

    #[test]
    fn counter_clamp_min() {
        let (mut store, _dir) = setup_store(test_schema());
        // warmth has min=0, default=0
        assert_eq!(store.dec("warmth", 100).unwrap(), 0);
    }

    #[test]
    fn counter_clamp_max() {
        let (mut store, _dir) = setup_store(test_schema());
        // capped has min=0, max=100, default=50
        assert_eq!(store.inc("capped", 1).unwrap(), 51);
        assert_eq!(store.inc("capped", 100).unwrap(), 100);
    }

    #[test]
    fn counter_set_and_clamp() {
        let (mut store, _dir) = setup_store(test_schema());
        store.set("capped", "200", None).unwrap();
        match store.get("capped").unwrap() {
            DataValue::Counter { value } => assert_eq!(*value, 100),
            _ => panic!("Expected counter"),
        }
    }

    // -- String operations --

    #[test]
    fn string_set_get() {
        let (mut store, _dir) = setup_store(test_schema());
        store.set("current_mood", "elated", None).unwrap();
        match store.get("current_mood").unwrap() {
            DataValue::String { value } => assert_eq!(value, "elated"),
            _ => panic!("Expected string"),
        }
    }

    // -- State operations --

    #[test]
    fn state_set_field() {
        let (mut store, _dir) = setup_store(test_schema());
        store.set("tensor", "0.75", Some("temperature")).unwrap();
        store.set("tensor", "0.30", Some("entropy")).unwrap();
        match store.get("tensor").unwrap() {
            DataValue::State { fields, .. } => {
                assert_eq!(fields["temperature"], "0.75");
                assert_eq!(fields["entropy"], "0.30");
            }
            _ => panic!("Expected state"),
        }
    }

    #[test]
    fn state_rejects_unknown_field() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.set("tensor", "0.5", Some("nonexistent"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown field"));
    }

    #[test]
    fn state_requires_field_name() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.set("tensor", "0.5", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("field name"));
    }

    // -- History operations --

    #[test]
    fn history_push_and_last() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store.push("flavor_history", "lapsang", None, None).unwrap();

        let last = store.last("flavor_history", 1, None, &[]).unwrap();
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].value, "lapsang");

        let last2 = store.last("flavor_history", 2, None, &[]).unwrap();
        assert_eq!(last2.len(), 2);
    }

    #[test]
    fn history_max_entries_overflow() {
        let (mut store, _dir) = setup_store(test_schema());
        // max_entries = 3
        store.push("flavor_history", "a", None, None).unwrap();
        store.push("flavor_history", "b", None, None).unwrap();
        store.push("flavor_history", "c", None, None).unwrap();
        store.push("flavor_history", "d", None, None).unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[0].value, "d"); // newest first
                assert_eq!(entries[2].value, "b"); // oldest kept
                // IDs should be assigned
                assert!(entries[0].id > 0);
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn history_since() {
        let (mut store, _dir) = setup_store(test_schema());

        let old_time = Utc::now() - chrono::Duration::hours(3);
        let new_time = Utc::now() - chrono::Duration::minutes(10);

        store
            .push_with_ts("flavor_history", "old_one", old_time, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "new_one", new_time, None, None)
            .unwrap();

        let results = store.since("flavor_history", "1h").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "new_one");
    }

    // -- List operations --

    #[test]
    fn list_push_pop_last() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        let last = store.last("tags", 2, None, &[]).unwrap();
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].value, "beta");
        assert_eq!(last[1].value, "gamma");

        let popped = store.pop("tags").unwrap();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().value, "gamma");
    }

    #[test]
    fn list_max_entries_overflow() {
        let (mut store, _dir) = setup_store(test_schema());
        // max_entries = 5
        for i in 0..8 {
            store
                .push("tags", &format!("item_{}", i), None, None)
                .unwrap();
        }
        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 5);
                assert_eq!(items[0].value, "item_3"); // oldest kept
                assert_eq!(items[4].value, "item_7"); // newest
            }
            _ => panic!("Expected list"),
        }
    }

    // -- Type mismatch --

    #[test]
    fn type_mismatch_inc_on_string() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.inc("current_mood", 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn type_mismatch_push_on_counter() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("warmth", "value", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn type_mismatch_pop_on_history() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.pop("flavor_history");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn type_mismatch_since_on_list() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.since("tags", "1h");
        assert!(result.is_err());
    }

    // -- Unknown key --

    #[test]
    fn unknown_key_rejected() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.get("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown key"));
    }

    // -- Reset --

    #[test]
    fn reset_counter() {
        let (mut store, _dir) = setup_store(test_schema());
        store.inc("warmth", 42).unwrap();
        store.reset("warmth").unwrap();
        match store.get("warmth").unwrap() {
            DataValue::Counter { value } => assert_eq!(*value, 0),
            _ => panic!("Expected counter"),
        }
    }

    #[test]
    fn reset_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "test", None, None).unwrap();
        store.reset("flavor_history").unwrap();
        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => assert!(entries.is_empty()),
            _ => panic!("Expected history"),
        }
    }

    // -- Keys listing --

    #[test]
    fn keys_lists_all() {
        let (store, _dir) = setup_store(test_schema());
        let keys = store.keys();
        assert_eq!(keys.len(), 6);
    }

    // -- Atomic write --

    #[test]
    fn atomic_write_round_trip() {
        let (mut store, _dir) = setup_store(test_schema());
        store.data.schema_id = "test".to_string();
        store.inc("warmth", 7).unwrap();
        store.set("current_mood", "happy", None).unwrap();
        store
            .push("flavor_history", "earl grey", None, None)
            .unwrap();
        store.save().unwrap();

        // Reload and verify
        let data_str = fs::read_to_string(&store.data_path).unwrap();
        let reloaded: DataFile = serde_json::from_str(&data_str).unwrap();
        assert_eq!(reloaded.schema_id, "test");
        match &reloaded.entries["warmth"] {
            DataValue::Counter { value } => assert_eq!(*value, 7),
            _ => panic!("Expected counter"),
        }
    }

    // -- Compact dump --

    #[test]
    fn compact_dump_format() {
        let (mut store, _dir) = setup_store(test_schema());
        store.inc("warmth", 42).unwrap();
        store.set("current_mood", "calm", None).unwrap();
        store.set("tensor", "0.55", Some("temperature")).unwrap();
        store.set("tensor", "0.35", Some("entropy")).unwrap();
        store.set("tensor", "0.70", Some("agency")).unwrap();

        let ts = DateTime::parse_from_rfc3339("2026-04-22T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.push_with_ts("tags", "focus", ts, None, None).unwrap();
        store.push_with_ts("tags", "rust", ts, None, None).unwrap();

        let compact = store.dump_compact();

        assert!(compact.contains("warmth=42"));
        assert!(compact.contains("current_mood=calm"));
        assert!(compact.contains("tensor={0.55,0.35,0.70}"));
        assert!(compact.contains("tags=[focus@10:30,rust@10:30]"));
    }

    #[test]
    fn compact_dump_counter_default() {
        let (store, _dir) = setup_store(test_schema());
        let compact = store.dump_compact();
        // Defaults when no data set
        assert!(compact.contains("warmth=0"));
        assert!(compact.contains("capped=50"));
    }

    #[test]
    fn compact_dump_history_with_times() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts = DateTime::parse_from_rfc3339("2026-04-22T19:13:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store
            .push_with_ts("flavor_history", "bergamot", ts, None, None)
            .unwrap();

        let compact = store.dump_compact();
        assert!(compact.contains("flavor_history=[bergamot@19:13]"));
    }

    // -- Relative time parser --

    #[test]
    fn parse_relative_minutes() {
        let result = parse_relative_time("30m").unwrap();
        let diff = Utc::now() - result;
        // Should be approximately 30 minutes
        assert!(diff.num_minutes() >= 29 && diff.num_minutes() <= 31);
    }

    #[test]
    fn parse_relative_hours() {
        let result = parse_relative_time("2h").unwrap();
        let diff = Utc::now() - result;
        assert!(diff.num_hours() >= 1 && diff.num_hours() <= 3);
    }

    #[test]
    fn parse_relative_days() {
        let result = parse_relative_time("7d").unwrap();
        let diff = Utc::now() - result;
        assert!(diff.num_days() >= 6 && diff.num_days() <= 8);
    }

    #[test]
    fn parse_relative_weeks() {
        let result = parse_relative_time("2w").unwrap();
        let diff = Utc::now() - result;
        assert!(diff.num_weeks() >= 1 && diff.num_weeks() <= 3);
    }

    #[test]
    fn parse_relative_invalid_unit() {
        assert!(parse_relative_time("5x").is_err());
    }

    #[test]
    fn parse_relative_invalid_number() {
        assert!(parse_relative_time("abch").is_err());
    }

    #[test]
    fn parse_relative_empty() {
        assert!(parse_relative_time("").is_err());
    }

    #[test]
    fn parse_timeref_iso8601() {
        let result = parse_timeref("2026-04-22T19:13:00Z").unwrap();
        assert_eq!(result.year(), 2026);
    }

    #[test]
    fn parse_timeref_relative_fallback() {
        let result = parse_timeref("1h").unwrap();
        let diff = Utc::now() - result;
        assert!(diff.num_hours() >= 0 && diff.num_hours() <= 2);
    }

    // -- Edge cases --

    #[test]
    fn inc_by_custom_amount() {
        let (mut store, _dir) = setup_store(test_schema());
        assert_eq!(store.inc("warmth", 10).unwrap(), 10);
        assert_eq!(store.dec("warmth", 3).unwrap(), 7);
    }

    #[test]
    fn last_on_empty_returns_empty() {
        let (store, _dir) = setup_store(test_schema());
        let last = store.last("flavor_history", 5, None, &[]).unwrap();
        assert!(last.is_empty());
    }

    #[test]
    fn pop_on_empty_returns_none() {
        let (store, _dir) = setup_store(test_schema());
        // Don't push anything — pop should be None on empty
        // Need mutable for pop
        let mut store = store;
        let popped = store.pop("tags").unwrap();
        assert!(popped.is_none());
    }

    // -- Remove operations --

    #[test]
    fn remove_list_by_value_first_match() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "alpha-2", None, None).unwrap();

        let result = store
            .remove("tags", Some("alpha"), None::<&IdRef>, false)
            .unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "alpha");

        // alpha-2 should still be there
        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].value, "beta");
                assert_eq!(items[1].value, "alpha-2");
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn remove_list_by_value_all_matches() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "alpha-2", None, None).unwrap();

        let result = store
            .remove("tags", Some("alpha"), None::<&IdRef>, true)
            .unwrap();
        assert_eq!(result.removed.len(), 2);

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].value, "beta");
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn remove_list_by_id() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        // Get the ID of "beta"
        let beta_id = match store.get("tags").unwrap() {
            DataValue::List { items, .. } => items[1].id,
            _ => panic!("Expected list"),
        };

        let result = store
            .remove("tags", None, Some(&IdRef::Numeric(beta_id)), false)
            .unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "beta");
    }

    #[test]
    fn remove_history_by_value() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store.push("flavor_history", "lapsang", None, None).unwrap();
        store
            .push("flavor_history", "bergamot vanilla", None, None)
            .unwrap();

        let result = store
            .remove("flavor_history", Some("bergamot"), None::<&IdRef>, false)
            .unwrap();
        assert_eq!(result.removed.len(), 1);

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries.len(), 2);
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn remove_case_insensitive() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "Alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();

        let result = store
            .remove("tags", Some("alpha"), None::<&IdRef>, false)
            .unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "Alpha");
    }

    #[test]
    fn remove_type_mismatch() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.remove("warmth", Some("x"), None::<&IdRef>, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- Search operations --

    #[test]
    fn search_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "rust-lang", None, None).unwrap();
        store.push("tags", "focus", None, None).unwrap();
        store.push("tags", "rust-tools", None, None).unwrap();

        let hits = store.search("tags", Some("rust"), None, &[]).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].value, "rust-lang");
        assert_eq!(hits[1].value, "rust-tools");
    }

    #[test]
    fn search_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store.push("flavor_history", "lapsang", None, None).unwrap();
        store
            .push("flavor_history", "bergamot vanilla", None, None)
            .unwrap();

        let hits = store
            .search("flavor_history", Some("bergamot"), None, &[])
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_case_insensitive() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "Rust", None, None).unwrap();
        store.push("tags", "RUST-tools", None, None).unwrap();
        store.push("tags", "python", None, None).unwrap();

        let hits = store.search("tags", Some("rust"), None, &[]).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_no_matches() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();

        let hits = store.search("tags", Some("zzz"), None, &[]).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_type_mismatch() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.search("warmth", Some("x"), None, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- get_entries_by_id operations --

    #[test]
    fn get_by_id_single_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store.push("flavor_history", "lapsang", None, None).unwrap();
        store
            .push("flavor_history", "earl grey", None, None)
            .unwrap();

        // Get the ID of the second entry
        let target_id = match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => entries[1].id,
            _ => panic!("Expected history"),
        };

        let hits = store
            .get_entries_by_id("flavor_history", &[IdRef::Numeric(target_id)])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, target_id);
        assert_eq!(hits[0].value, "lapsang");
    }

    #[test]
    fn get_by_id_single_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        // Read the actual ID of the second entry from the store
        let target_id = match store.get("tags").unwrap() {
            DataValue::List { items, .. } => items[1].id,
            _ => panic!("Expected list"),
        };

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(target_id)])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, target_id);
        assert_eq!(hits[0].value, "beta");
    }

    #[test]
    fn get_by_id_range_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        let hits = store
            .get_entries_by_id(
                "tags",
                &[IdRef::Numeric(1), IdRef::Numeric(2), IdRef::Numeric(3)],
            )
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].value, "alpha");
        assert_eq!(hits[1].value, "beta");
        assert_eq!(hits[2].value, "gamma");
    }

    #[test]
    fn get_by_id_multi_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(1), IdRef::Numeric(3)])
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].value, "alpha");
        assert_eq!(hits[1].value, "gamma");
    }

    #[test]
    fn get_by_id_partial_match() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();

        // Request IDs 1, 2, 99 — only 1 and 2 exist
        let hits = store
            .get_entries_by_id(
                "tags",
                &[IdRef::Numeric(1), IdRef::Numeric(2), IdRef::Numeric(99)],
            )
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn get_by_id_all_not_found() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(99), IdRef::Numeric(100)])
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn get_by_id_type_mismatch_counter() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.get_entries_by_id("warmth", &[IdRef::Numeric(1)]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn get_by_id_type_mismatch_string() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.get_entries_by_id("current_mood", &[IdRef::Numeric(1)]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn get_by_id_type_mismatch_state() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.get_entries_by_id("tensor", &[IdRef::Numeric(1)]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn get_by_id_empty_history() {
        let (store, _dir) = setup_store(test_schema());
        let hits = store
            .get_entries_by_id("flavor_history", &[IdRef::Numeric(1)])
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn get_by_id_empty_list() {
        let (store, _dir) = setup_store(test_schema());
        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(1)])
            .unwrap();
        assert!(hits.is_empty());
    }

    // -- Count operations --

    #[test]
    fn count_list_total() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "a", None, None).unwrap();
        store.push("tags", "b", None, None).unwrap();
        store.push("tags", "c", None, None).unwrap();

        let result = store.count("tags", None, None, &[]).unwrap();
        assert_eq!(result.matched, 3);
        assert!(result.total.is_none()); // no filter => no total
    }

    #[test]
    fn count_list_filtered() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "rust-lang", None, None).unwrap();
        store.push("tags", "focus", None, None).unwrap();
        store.push("tags", "rust-tools", None, None).unwrap();

        let result = store.count("tags", Some("rust"), None, &[]).unwrap();
        assert_eq!(result.matched, 2);
        assert_eq!(result.total, Some(3)); // 2 of 3 match
    }

    #[test]
    fn count_history_with_latest() {
        let (mut store, _dir) = setup_store(test_schema());
        let ts1 = DateTime::parse_from_rfc3339("2026-04-20T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts2 = DateTime::parse_from_rfc3339("2026-04-22T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "bergamot", ts1, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "bergamot vanilla", ts2, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "lapsang", ts1, None, None)
            .unwrap();

        let result = store
            .count("flavor_history", Some("bergamot"), None, &[])
            .unwrap();
        assert_eq!(result.matched, 2);
        assert_eq!(result.total, Some(3)); // 2 of 3 match
        assert!(result.latest_ts.is_some());
        assert!(result.latest_ts.unwrap().contains("2026-04-22"));
    }

    #[test]
    fn count_empty() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.count("tags", None, None, &[]).unwrap();
        assert_eq!(result.matched, 0);
        assert!(result.total.is_none());
        assert!(result.latest_ts.is_none());
    }

    #[test]
    fn count_filtered_empty_total() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.count("tags", Some("rust"), None, &[]).unwrap();
        assert_eq!(result.matched, 0);
        assert_eq!(result.total, Some(0)); // 0 of 0
    }

    #[test]
    fn count_type_mismatch() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.count("warmth", None, None, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- ID assignment --

    #[test]
    fn history_ids_auto_increment() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "a", None, None).unwrap();
        store.push("flavor_history", "b", None, None).unwrap();
        store.push("flavor_history", "c", None, None).unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                // Each push should get a unique ID
                let ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
                assert_eq!(ids.len(), 3);
                // All unique
                let mut unique = ids.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(unique.len(), 3);
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn list_ids_auto_increment() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "a", None, None).unwrap();
        store.push("tags", "b", None, None).unwrap();
        store.push("tags", "c", None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].id, 1);
                assert_eq!(items[1].id, 2);
                assert_eq!(items[2].id, 3);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn list_ids_stable_after_remove() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "a", None, None).unwrap();
        store.push("tags", "b", None, None).unwrap();
        store.push("tags", "c", None, None).unwrap();

        // Remove "b"
        store
            .remove("tags", Some("b"), None::<&IdRef>, false)
            .unwrap();

        // Push "d" — should get id=4, not reuse id=2
        store.push("tags", "d", None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].id, 1); // a
                assert_eq!(items[1].id, 3); // c
                assert_eq!(items[2].id, 4); // d
            }
            _ => panic!("Expected list"),
        }
    }

    // -- Backward compatibility --

    #[test]
    fn deserialize_old_bare_string_list() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Write old-format data file with bare string list
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "tags": { "items": ["alpha", "beta", "gamma"] }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].value, "alpha");
                assert_eq!(items[1].value, "beta");
                assert_eq!(items[2].value, "gamma");
                // Should have auto-assigned IDs
                assert!(items[0].id > 0);
                assert!(items[1].id > items[0].id);
                // Timestamps should be empty (placeholder)
                assert!(items[0].ts.is_empty());
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn deserialize_old_history_without_id() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Write old-format data file with history entries lacking id
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "flavor_history": {
                "entries": [
                    {"value": "bergamot", "ts": "2026-04-22T19:13:00Z"},
                    {"value": "lapsang", "ts": "2026-04-21T10:00:00Z"}
                ]
            }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].value, "bergamot");
                // Should have auto-assigned IDs
                assert!(entries[0].id > 0);
                assert!(entries[1].id > 0);
                assert_ne!(entries[0].id, entries[1].id);
            }
            _ => panic!("Expected history"),
        }
    }

    // -- ListEntry timestamps --

    #[test]
    fn list_push_assigns_timestamp() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts = DateTime::parse_from_rfc3339("2026-04-22T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.push_with_ts("tags", "focus", ts, None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 1);
                assert!(items[0].ts.contains("2026-04-22"));
                assert_eq!(items[0].value, "focus");
            }
            _ => panic!("Expected list"),
        }
    }

    // -- format_value output --

    #[test]
    fn format_value_list_shows_ids_and_timestamps() {
        let (mut store, _dir) = setup_store(test_schema());
        let ts = DateTime::parse_from_rfc3339("2026-04-22T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.push_with_ts("tags", "focus", ts, None, None).unwrap();
        store.push_with_ts("tags", "rust", ts, None, None).unwrap();

        let output = format_value(store.get("tags").unwrap());
        // New format: "1 [kv-XXXX]: focus (ts)"
        assert!(output.contains("1 [kv-"));
        assert!(output.contains("]: focus"));
        assert!(output.contains("2 [kv-"));
        assert!(output.contains("]: rust"));
        assert!(output.contains("2026-04-22"));
    }

    #[test]
    fn format_value_history_shows_ids() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();

        let output = format_value(store.get("flavor_history").unwrap());
        // Should contain "N: bergamot (timestamp)"
        assert!(output.contains("bergamot"));
        assert!(output.contains(":"));
    }

    // -- compact dump with list timestamps --

    #[test]
    fn compact_dump_list_with_times() {
        let (mut store, _dir) = setup_store(test_schema());
        let ts = DateTime::parse_from_rfc3339("2026-04-22T14:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store
            .push_with_ts("tags", "dsi-panel", ts, None, None)
            .unwrap();
        store
            .push_with_ts("tags", "anytype", ts, None, None)
            .unwrap();

        let compact = store.dump_compact();
        assert!(compact.contains("tags=[dsi-panel@14:15,anytype@14:15]"));
    }

    use chrono::Datelike as _;

    // -- Memory pointer operations --

    #[test]
    fn set_get_memory_on_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();

        // Initially no memory pointer
        assert_eq!(store.get_memory("flavor_history").unwrap(), None);

        // Set a memory pointer
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();
        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );

        // Verify it persists through save/reload
        store.data.schema_id = "test".to_string();
        store.save().unwrap();
        let data_str = fs::read_to_string(&store.data_path).unwrap();
        assert!(data_str.contains("kn-abc123"));
    }

    #[test]
    fn set_get_memory_on_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();

        store
            .set_memory("tags", Some("kn-def456".to_string()))
            .unwrap();
        assert_eq!(store.get_memory("tags").unwrap(), Some("kn-def456"));
    }

    #[test]
    fn set_get_memory_on_state() {
        let (mut store, _dir) = setup_store(test_schema());
        store.set("tensor", "0.5", Some("temperature")).unwrap();

        store
            .set_memory("tensor", Some("kn-789ghi".to_string()))
            .unwrap();
        assert_eq!(store.get_memory("tensor").unwrap(), Some("kn-789ghi"));
    }

    #[test]
    fn set_memory_on_counter_rejected() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.set_memory("warmth", Some("kn-abc".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn set_memory_on_string_rejected() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.set_memory("current_mood", Some("kn-abc".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn get_memory_on_counter_rejected() {
        let (mut store, _dir) = setup_store(test_schema());
        store.inc("warmth", 1).unwrap();
        let result = store.get_memory("warmth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn clear_memory_with_empty_string() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();

        // Set then clear
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();
        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );

        // Clear with empty string
        store
            .set_memory("flavor_history", Some("".to_string()))
            .unwrap();
        assert_eq!(store.get_memory("flavor_history").unwrap(), None);
    }

    #[test]
    fn clear_memory_with_none() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();

        store
            .set_memory("tags", Some("kn-abc123".to_string()))
            .unwrap();
        store.set_memory("tags", None).unwrap();
        assert_eq!(store.get_memory("tags").unwrap(), None);
    }

    #[test]
    fn memory_not_serialized_when_none() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store.data.schema_id = "test".to_string();
        store.save().unwrap();

        let data_str = fs::read_to_string(&store.data_path).unwrap();
        // "memory" should NOT appear in JSON when it's None
        assert!(!data_str.contains("\"memory\""));
    }

    #[test]
    fn backward_compat_old_data_without_memory() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Old-format data file without memory field
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "flavor_history": {
                "entries": [
                    {"id": 1, "value": "bergamot", "ts": "2026-04-22T19:13:00Z"}
                ]
            },
            "tags": {
                "items": [
                    {"id": 1, "value": "focus", "ts": "2026-04-22T10:30:00Z"}
                ]
            },
            "tensor": {
                "fields": {"temperature": "0.55", "entropy": "0.35", "agency": "0.70"}
            }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();

        // All should deserialize cleanly with no memory pointer
        assert_eq!(store.get_memory("flavor_history").unwrap(), None);
        assert_eq!(store.get_memory("tags").unwrap(), None);
        assert_eq!(store.get_memory("tensor").unwrap(), None);

        // Data should still be accessible
        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].value, "bergamot");
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn backward_compat_data_with_memory() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Data file WITH memory field
        let data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "flavor_history": {
                "entries": [
                    {"id": 1, "value": "bergamot", "ts": "2026-04-22T19:13:00Z"}
                ],
                "memory": "kn-abc123"
            },
            "tags": {
                "items": [
                    {"id": 1, "value": "focus", "ts": "2026-04-22T10:30:00Z"}
                ],
                "memory": "kn-def456"
            }
        }"#;
        fs::write(&data_path, data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();

        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );
        assert_eq!(store.get_memory("tags").unwrap(), Some("kn-def456"));
    }

    // -- Compact dump with memory pointers --

    #[test]
    fn compact_dump_with_memory_pointer() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts = DateTime::parse_from_rfc3339("2026-04-22T19:13:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store
            .push_with_ts("flavor_history", "bergamot", ts, None, None)
            .unwrap();
        store
            .set_memory("flavor_history", Some("kn-def456".to_string()))
            .unwrap();

        store.set("tensor", "0.55", Some("temperature")).unwrap();
        store.set("tensor", "0.35", Some("entropy")).unwrap();
        store.set("tensor", "0.70", Some("agency")).unwrap();
        store
            .set_memory("tensor", Some("kn-789ghi".to_string()))
            .unwrap();

        let compact = store.dump_compact();
        assert!(compact.contains("flavor_history=[bergamot@19:13](kn-def456)"));
        assert!(compact.contains("tensor={0.55,0.35,0.70}(kn-789ghi)"));
    }

    #[test]
    fn compact_dump_without_memory_pointer() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts = DateTime::parse_from_rfc3339("2026-04-22T19:13:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store
            .push_with_ts("flavor_history", "bergamot", ts, None, None)
            .unwrap();
        // No memory pointer set

        let compact = store.dump_compact();
        assert!(compact.contains("flavor_history=[bergamot@19:13]"));
        // Should NOT contain parenthetical
        assert!(!compact.contains("flavor_history=[bergamot@19:13]("));
    }

    #[test]
    fn set_memory_on_nonexistent_data_creates_default() {
        let (mut store, _dir) = setup_store(test_schema());

        // No data for flavor_history yet, but set memory should initialize it
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();
        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );

        // Should have created default empty history
        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => assert!(entries.is_empty()),
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn memory_survives_push() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();

        // Push another entry
        store.push("flavor_history", "lapsang", None, None).unwrap();

        // Memory pointer should still be set
        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );
    }

    #[test]
    fn memory_survives_reset() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();

        // Reset clears data to default — memory pointer should be cleared too
        store.reset("flavor_history").unwrap();

        // After reset, memory is gone (default_value has memory: None)
        assert_eq!(store.get_memory("flavor_history").unwrap(), None);
    }

    // -----------------------------------------------------------------
    // Migration: ~/.crewu/kv -> $MX_HOME/kv (decision 1)
    // -----------------------------------------------------------------

    #[test]
    fn kv_path_env_override_wins() {
        let (path, warn) = resolve_kv_path_with(
            Some("/explicit/{agent}.toml"),
            "smith",
            std::path::PathBuf::from("/new/smith.toml"),
            Some(std::path::PathBuf::from("/legacy/smith.toml")),
        );
        assert_eq!(path, std::path::PathBuf::from("/explicit/smith.toml"));
        assert!(!warn);
    }

    #[test]
    fn kv_path_uses_new_default_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let new_path = dir.path().join("new.toml");
        std::fs::write(&new_path, "").unwrap();
        let legacy = dir.path().join("legacy.toml");
        std::fs::write(&legacy, "").unwrap();

        let (path, warn) = resolve_kv_path_with(None, "smith", new_path.clone(), Some(legacy));
        assert_eq!(path, new_path);
        assert!(!warn);
    }

    #[test]
    fn kv_path_falls_back_to_legacy_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let new_path = dir.path().join("missing.toml"); // doesn't exist
        let legacy = dir.path().join("legacy.toml");
        std::fs::write(&legacy, "").unwrap();

        let (path, warn) = resolve_kv_path_with(None, "smith", new_path, Some(legacy.clone()));
        assert_eq!(path, legacy);
        assert!(warn, "warning MUST fire on legacy fallback");
    }

    #[test]
    fn kv_path_returns_new_default_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        let new_path = dir.path().join("missing.toml");
        let legacy = dir.path().join("also-missing.toml");

        let (path, warn) = resolve_kv_path_with(None, "smith", new_path.clone(), Some(legacy));
        assert_eq!(path, new_path);
        assert!(!warn);
    }

    #[test]
    fn kv_path_empty_env_treated_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let new_path = dir.path().join("new.toml");
        std::fs::write(&new_path, "").unwrap();
        let (path, warn) = resolve_kv_path_with(Some(""), "smith", new_path.clone(), None);
        assert_eq!(path, new_path);
        assert!(!warn);
    }

    // -----------------------------------------------------------------
    // should_emit_legacy_kv_warning -- dedupe gate (Critical 2 fix)
    // -----------------------------------------------------------------

    #[test]
    fn legacy_warning_silent_when_no_fallback() {
        let gate = std::sync::OnceLock::new();
        assert!(!should_emit_legacy_kv_warning(false, false, &gate));
        // gate must remain unset so a later real fallback still warns
        assert!(gate.get().is_none());
    }

    #[test]
    fn legacy_warning_fires_once_when_only_schema_warns() {
        let gate = std::sync::OnceLock::new();
        assert!(should_emit_legacy_kv_warning(true, false, &gate));
        assert!(!should_emit_legacy_kv_warning(true, false, &gate));
    }

    #[test]
    fn legacy_warning_fires_once_when_only_data_warns() {
        let gate = std::sync::OnceLock::new();
        assert!(should_emit_legacy_kv_warning(false, true, &gate));
        assert!(!should_emit_legacy_kv_warning(false, true, &gate));
    }

    #[test]
    fn legacy_warning_fires_once_when_both_warn() {
        // Models the Critical-2 scenario: both schema and data legacy-fallback
        // in a single `from_env` call -- user must see ONE warning, not two.
        let gate = std::sync::OnceLock::new();
        assert!(should_emit_legacy_kv_warning(true, true, &gate));
        // Subsequent calls (e.g. another resolver in the same process) stay quiet.
        assert!(!should_emit_legacy_kv_warning(true, true, &gate));
        assert!(!should_emit_legacy_kv_warning(false, true, &gate));
        assert!(!should_emit_legacy_kv_warning(true, false, &gate));
    }

    // -----------------------------------------------------------------
    // Production resolvers -- exercise the actual code path that ships
    // (Critical 1 fix). These touch env vars so they share a mutex.
    // -----------------------------------------------------------------

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: sets an env var on construction, restores on drop.
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: tests serialize on ENV_LOCK before constructing guards.
            unsafe {
                std::env::set_var(key, val);
            }
            Self { key, prior }
        }

        fn unset(key: &'static str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: see above.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests serialize on ENV_LOCK; this runs while guard is held.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn production_resolve_schema_path_honors_env_with_agent_substitution() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set("MX_KV_SCHEMA", "/tmp/explicit/{agent}.toml");
        let (path, warn) = KvStore::resolve_schema_path("smith");
        assert_eq!(path, std::path::PathBuf::from("/tmp/explicit/smith.toml"));
        assert!(!warn);
    }

    #[test]
    fn production_resolve_data_path_honors_env_with_agent_substitution() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set("MX_KV_DATA", "/tmp/explicit/{agent}.json");
        let (path, warn) = KvStore::resolve_data_path("smith");
        assert_eq!(path, std::path::PathBuf::from("/tmp/explicit/smith.json"));
        assert!(!warn);
    }

    #[test]
    fn production_resolve_schema_path_empty_env_is_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set("MX_KV_SCHEMA", "");
        // Empty env -> resolver should NOT return PathBuf::from("") -- it
        // should fall through to the default-derived path. The exact path
        // depends on which file (if any) exists on the test host -- so we
        // assert against the canonical helpers, not string literals.
        let (path, _warn) = KvStore::resolve_schema_path("smith");
        assert_ne!(path, std::path::PathBuf::from(""));
        let new_default = crate::paths::kv_schema_path("smith");
        let legacy = crate::paths::legacy_crewu_kv_schema_path("smith");
        assert!(
            path == new_default || legacy.as_ref() == Some(&path),
            "expected one of the derived paths, got: {}",
            path.display()
        );
    }

    #[test]
    fn production_resolve_data_path_empty_env_is_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set("MX_KV_DATA", "");
        let (path, _warn) = KvStore::resolve_data_path("smith");
        assert_ne!(path, std::path::PathBuf::from(""));
        let new_default = crate::paths::kv_data_path("smith");
        let legacy = crate::paths::legacy_crewu_kv_data_path("smith");
        assert!(
            path == new_default || legacy.as_ref() == Some(&path),
            "expected one of the derived paths, got: {}",
            path.display()
        );
    }

    #[test]
    fn production_resolve_schema_path_returns_default_when_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::unset("MX_KV_SCHEMA");
        let (path, _warn) = KvStore::resolve_schema_path("smith");
        // Either the new $MX_HOME default OR the legacy fallback (if it
        // happens to exist on the test host). Both are acceptable -- we
        // assert against the canonical helpers so this test never hardcodes
        // a string literal that the path-alignment grep guards forbid.
        let new_default = crate::paths::kv_schema_path("smith");
        let legacy = crate::paths::legacy_crewu_kv_schema_path("smith");
        assert!(
            path == new_default || legacy.as_ref() == Some(&path),
            "unexpected default path: {}",
            path.display()
        );
    }

    // -----------------------------------------------------------------
    // Time-range parsing
    // -----------------------------------------------------------------

    #[test]
    fn parse_day_valid() {
        let range = parse_day("2026-04-25").unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-25");
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-04-26");
    }

    #[test]
    fn parse_day_invalid_format() {
        assert!(parse_day("04-25-2026").is_err());
        assert!(parse_day("2026/04/25").is_err());
        assert!(parse_day("not-a-date").is_err());
    }

    #[test]
    fn parse_month_valid() {
        let range = parse_month("2026-04").unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-01");
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-05-01");
    }

    #[test]
    fn parse_month_december_rollover() {
        let range = parse_month("2026-12").unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-12-01");
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2027-01-01");
    }

    #[test]
    fn parse_month_invalid_format() {
        assert!(parse_month("2026").is_err());
        assert!(parse_month("2026-13").is_err());
        assert!(parse_month("2026-00").is_err());
        assert!(parse_month("not-a-month").is_err());
    }

    #[test]
    fn parse_week_valid() {
        let range = parse_week("2026-W17").unwrap();
        // ISO week 17 of 2026 starts on Monday 2026-04-20
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-20");
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-04-27");
    }

    #[test]
    fn parse_week_1() {
        let range = parse_week("2026-W01").unwrap();
        // ISO week 1 of 2026 starts on Monday 2025-12-29
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2025-12-29");
        let diff = range.to - range.from;
        assert_eq!(diff.num_days(), 7);
    }

    #[test]
    fn parse_week_invalid_format() {
        assert!(parse_week("2026-17").is_err());
        assert!(parse_week("2026-W00").is_err());
        assert!(parse_week("not-a-week").is_err());
    }

    #[test]
    fn parse_date_range_both_provided() {
        let range = parse_date_range(Some("2026-04-01"), Some("2026-04-15")).unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-01");
        // to is inclusive of the end date, so upper bound is start of next day
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-04-16");
    }

    #[test]
    fn parse_date_range_from_only() {
        let range = parse_date_range(Some("2026-04-01"), None).unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-01");
        // to defaults to now, so it should be roughly today
        let diff = Utc::now() - range.to;
        assert!(diff.num_seconds().abs() < 5);
    }

    #[test]
    fn parse_date_range_to_only() {
        let range = parse_date_range(None, Some("2026-04-15")).unwrap();
        assert_eq!(range.from, DateTime::UNIX_EPOCH);
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-04-16");
    }

    #[test]
    fn parse_date_range_from_after_to_errors() {
        let result = parse_date_range(Some("2026-04-15"), Some("2026-04-01"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_time_range_no_flags() {
        let args = TimeRangeArgs::default();
        assert!(resolve_time_range(&args).unwrap().is_none());
    }

    #[test]
    fn resolve_time_range_day() {
        let args = TimeRangeArgs {
            day: Some("2026-04-25".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap().unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-25");
    }

    #[test]
    fn resolve_time_range_month() {
        let args = TimeRangeArgs {
            month: Some("2026-04".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap().unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-01");
    }

    #[test]
    fn resolve_time_range_week() {
        let args = TimeRangeArgs {
            week: Some("2026-W17".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap().unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-20");
    }

    #[test]
    fn resolve_time_range_from_to() {
        let args = TimeRangeArgs {
            range_from: Some("2026-04-01".to_string()),
            range_to: Some("2026-04-15".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap().unwrap();
        assert_eq!(range.from.format("%Y-%m-%d").to_string(), "2026-04-01");
        assert_eq!(range.to.format("%Y-%m-%d").to_string(), "2026-04-16");
    }

    // -----------------------------------------------------------------
    // ts_in_range
    // -----------------------------------------------------------------

    #[test]
    fn ts_in_range_inside() {
        let range = parse_day("2026-04-25").unwrap();
        assert!(ts_in_range("2026-04-25T12:00:00+00:00", &range));
    }

    #[test]
    fn ts_in_range_at_lower_boundary() {
        let range = parse_day("2026-04-25").unwrap();
        assert!(ts_in_range("2026-04-25T00:00:00+00:00", &range));
    }

    #[test]
    fn ts_in_range_at_upper_boundary_excluded() {
        let range = parse_day("2026-04-25").unwrap();
        // Upper bound is exclusive, so midnight of the next day is out
        assert!(!ts_in_range("2026-04-26T00:00:00+00:00", &range));
    }

    #[test]
    fn ts_in_range_outside() {
        let range = parse_day("2026-04-25").unwrap();
        assert!(!ts_in_range("2026-04-24T23:59:59+00:00", &range));
        assert!(!ts_in_range("2026-04-26T00:00:01+00:00", &range));
    }

    #[test]
    fn ts_in_range_empty_ts() {
        let range = parse_day("2026-04-25").unwrap();
        assert!(!ts_in_range("", &range));
    }

    #[test]
    fn ts_in_range_invalid_ts() {
        let range = parse_day("2026-04-25").unwrap();
        assert!(!ts_in_range("not-a-timestamp", &range));
    }

    // -----------------------------------------------------------------
    // Time-range integration with last/search/count
    // -----------------------------------------------------------------

    #[test]
    fn last_with_time_range_filters_history() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "inside", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "outside", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let results = store.last("flavor_history", 10, Some(&range), &[]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "inside");
    }

    #[test]
    fn last_with_time_range_composes_with_count() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "a", ts, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "b", ts, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        // Both entries match the range, but count=1 limits output
        let results = store.last("flavor_history", 1, Some(&range), &[]).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_with_time_range_filters() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("tags", "rust-in", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("tags", "rust-out", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let hits = store
            .search("tags", Some("rust"), Some(&range), &[])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "rust-in");
    }

    #[test]
    fn last_with_time_range_filters_list() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("tags", "inside", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("tags", "outside", ts_out, None, None)
            .unwrap();
        store
            .push_with_ts("tags", "also-inside", ts_in, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let results = store.last("tags", 10, Some(&range), &[]).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].value, "inside");
        assert_eq!(results[1].value, "also-inside");

        // count limit applies after time filter
        let results = store.last("tags", 1, Some(&range), &[]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "also-inside");
    }

    #[test]
    fn search_with_time_range_filters_history() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "bergamot-in", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "bergamot-out", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let hits = store
            .search("flavor_history", Some("bergamot"), Some(&range), &[])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "bergamot-in");
    }

    #[test]
    fn count_with_time_range_filters() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "bergamot", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "lapsang", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "bergamot earl", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        // Count all in range (no value filter)
        let result = store
            .count("flavor_history", None, Some(&range), &[])
            .unwrap();
        assert_eq!(result.matched, 2);

        // Count with value filter + time range
        let result = store
            .count("flavor_history", Some("bergamot"), Some(&range), &[])
            .unwrap();
        assert_eq!(result.matched, 1);
        assert_eq!(result.total, Some(2)); // 1 of 2 in-range entries match
    }

    #[test]
    fn count_with_month_range() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_apr = DateTime::parse_from_rfc3339("2026-04-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_may = DateTime::parse_from_rfc3339("2026-05-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "april", ts_apr, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "may", ts_may, None, None)
            .unwrap();

        let range = parse_month("2026-04").unwrap();
        let result = store
            .count("flavor_history", None, Some(&range), &[])
            .unwrap();
        assert_eq!(result.matched, 1);
    }

    // -- Random sampling --

    #[test]
    fn random_on_empty_returns_empty() {
        let (store, _dir) = setup_store(test_schema());
        let results = store.random("flavor_history", 5, None, &[]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn random_returns_requested_count() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        let results = store.random("tags", 2, None, &[]).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn random_returns_all_when_count_exceeds_available() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();

        let results = store.random("tags", 10, None, &[]).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn random_type_mismatch_on_counter() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.random("warmth", 1, None, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            KvError::TypeMismatch { key, .. } => assert_eq!(key, "warmth"),
            other => panic!("Expected TypeMismatch, got {:?}", other),
        }
    }

    #[test]
    fn random_works_on_history() {
        let (mut store, _dir) = setup_store(test_schema());
        let ts = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "bergamot", ts, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "vanilla", ts, None, None)
            .unwrap();

        let results = store.random("flavor_history", 1, None, &[]).unwrap();
        assert_eq!(results.len(), 1);
        // Result must be one of the pushed values
        assert!(results[0].value == "bergamot" || results[0].value == "vanilla");
    }

    #[test]
    fn random_with_time_range_filters() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-20T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("tags", "inside", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("tags", "outside", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let results = store.random("tags", 10, Some(&range), &[]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "inside");
    }

    #[test]
    fn random_with_time_range_filters_history() {
        let (mut store, _dir) = setup_store(test_schema());

        let ts_in = DateTime::parse_from_rfc3339("2026-04-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts_out = DateTime::parse_from_rfc3339("2026-04-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store
            .push_with_ts("flavor_history", "inside", ts_in, None, None)
            .unwrap();
        store
            .push_with_ts("flavor_history", "outside", ts_out, None, None)
            .unwrap();

        let range = parse_day("2026-04-25").unwrap();
        let results = store
            .random("flavor_history", 10, Some(&range), &[])
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "inside");
    }

    // -- --since resolver --

    #[test]
    fn resolve_time_range_since_relative() {
        let args = TimeRangeArgs {
            since: Some("1h".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap();
        assert!(range.is_some());
        let range = range.unwrap();
        // "since 1h" means from ~1 hour ago to now
        let one_hour_ago = Utc::now() - chrono::Duration::hours(1);
        // Allow a few seconds of drift
        assert!((range.from - one_hour_ago).num_seconds().abs() < 5);
        assert!((range.to - Utc::now()).num_seconds().abs() < 5);
    }

    #[test]
    fn resolve_time_range_since_days() {
        let args = TimeRangeArgs {
            since: Some("30d".to_string()),
            ..Default::default()
        };
        let range = resolve_time_range(&args).unwrap();
        assert!(range.is_some());
        let range = range.unwrap();
        let thirty_days_ago = Utc::now() - chrono::Duration::days(30);
        assert!((range.from - thirty_days_ago).num_seconds().abs() < 5);
    }

    // -----------------------------------------------------------------------
    // Structured data tests
    // -----------------------------------------------------------------------

    #[test]
    fn push_with_data_stores_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"status": "active", "tags": ["rust", "kv"]});
        store
            .push("tags", "my-item", Some(data.clone()), None)
            .unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].value, "my-item");
                assert_eq!(items[0].data, Some(data));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn push_without_data_stores_none() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "bare-item", None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 1);
                assert!(items[0].data.is_none());
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn push_history_with_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"mood": "focused"});
        store
            .push("flavor_history", "bergamot", Some(data.clone()), None)
            .unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries[0].data, Some(data));
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn data_round_trip_through_save() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"priority": 1, "tags": ["a", "b"]});
        store
            .push("tags", "item", Some(data.clone()), None)
            .unwrap();
        store.data.schema_id = "test".to_string();
        store.save().unwrap();

        // Reload
        let data_str = fs::read_to_string(&store.data_path).unwrap();
        let reloaded: DataFile = serde_json::from_str(&data_str).unwrap();
        match &reloaded.entries["tags"] {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].data, Some(data));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn backward_compat_missing_data_field() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Write data without any "data" fields
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "tags": { "items": [{"id": 1, "value": "alpha", "ts": "2026-04-20T00:00:00Z"}] },
            "flavor_history": { "entries": [{"id": 1, "value": "bergamot", "ts": "2026-04-20T00:00:00Z"}] }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();

        // List entry should have data=None
        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert!(items[0].data.is_none());
            }
            _ => panic!("Expected list"),
        }

        // History entry should have data=None
        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert!(entries[0].data.is_none());
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn backward_compat_bare_string_list_gets_none_data() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-04-20T00:00:00Z",
            "tags": { "items": ["alpha", "beta"] }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();
        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 2);
                assert!(items[0].data.is_none());
                assert!(items[1].data.is_none());
            }
            _ => panic!("Expected list"),
        }
    }

    // -- where_matches unit tests --

    #[test]
    fn where_matches_empty_clauses() {
        assert!(where_matches(&None, &[]));
        assert!(where_matches(&Some(serde_json::json!({"a": "b"})), &[]));
    }

    #[test]
    fn where_matches_none_data_nonempty_clauses() {
        let clauses = vec![("status".to_string(), "active".to_string())];
        assert!(!where_matches(&None, &clauses));
    }

    #[test]
    fn where_matches_exact_string() {
        let data = Some(serde_json::json!({"status": "active", "priority": "high"}));
        let clauses = vec![("status".to_string(), "active".to_string())];
        assert!(where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_exact_string_no_match() {
        let data = Some(serde_json::json!({"status": "closed"}));
        let clauses = vec![("status".to_string(), "active".to_string())];
        assert!(!where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_array_contains() {
        let data = Some(serde_json::json!({"tags": ["rust", "cli", "kv"]}));
        let clauses = vec![("tags".to_string(), "rust".to_string())];
        assert!(where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_array_not_contains() {
        let data = Some(serde_json::json!({"tags": ["rust", "cli"]}));
        let clauses = vec![("tags".to_string(), "python".to_string())];
        assert!(!where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_multiple_clauses_and_logic() {
        let data = Some(serde_json::json!({"status": "active", "priority": "high"}));

        // Both match
        let clauses = vec![
            ("status".to_string(), "active".to_string()),
            ("priority".to_string(), "high".to_string()),
        ];
        assert!(where_matches(&data, &clauses));

        // One fails
        let clauses = vec![
            ("status".to_string(), "active".to_string()),
            ("priority".to_string(), "low".to_string()),
        ];
        assert!(!where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_missing_key() {
        let data = Some(serde_json::json!({"status": "active"}));
        let clauses = vec![("nonexistent".to_string(), "value".to_string())];
        assert!(!where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_number_as_string() {
        let data = Some(serde_json::json!({"count": 42}));
        let clauses = vec![("count".to_string(), "42".to_string())];
        assert!(where_matches(&data, &clauses));
    }

    #[test]
    fn where_matches_bool_as_string() {
        let data = Some(serde_json::json!({"active": true}));
        let clauses = vec![("active".to_string(), "true".to_string())];
        assert!(where_matches(&data, &clauses));
    }

    // -- search with where_clauses --

    #[test]
    fn search_with_where_exact_match() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "task-1",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "task-2",
                Some(serde_json::json!({"status": "closed"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "task-3",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        let hits = store.search("tags", None, None, &clauses).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].value, "task-1");
        assert_eq!(hits[1].value, "task-3");
    }

    #[test]
    fn search_with_where_array_contains() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "item-a",
                Some(serde_json::json!({"labels": ["bug", "urgent"]})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "item-b",
                Some(serde_json::json!({"labels": ["feature"]})),
                None,
            )
            .unwrap();

        let clauses = vec![("labels".to_string(), "bug".to_string())];
        let hits = store.search("tags", None, None, &clauses).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "item-a");
    }

    #[test]
    fn search_with_where_excludes_entries_without_data() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "no-data", None, None).unwrap();
        store
            .push(
                "tags",
                "has-data",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        let hits = store.search("tags", None, None, &clauses).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "has-data");
    }

    #[test]
    fn search_with_query_and_where() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "rust-fix",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "rust-feature",
                Some(serde_json::json!({"status": "closed"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "python-fix",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        // Text query "rust" AND where status=active
        let hits = store.search("tags", Some("rust"), None, &clauses).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "rust-fix");
    }

    #[test]
    fn search_with_only_where_no_query() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("tags", "a", Some(serde_json::json!({"type": "bug"})), None)
            .unwrap();
        store
            .push(
                "tags",
                "b",
                Some(serde_json::json!({"type": "feature"})),
                None,
            )
            .unwrap();

        let clauses = vec![("type".to_string(), "bug".to_string())];
        let hits = store.search("tags", None, None, &clauses).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "a");
    }

    #[test]
    fn search_multiple_where_clauses_and() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "match-both",
                Some(serde_json::json!({"status": "active", "priority": "high"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "match-one",
                Some(serde_json::json!({"status": "active", "priority": "low"})),
                None,
            )
            .unwrap();

        let clauses = vec![
            ("status".to_string(), "active".to_string()),
            ("priority".to_string(), "high".to_string()),
        ];
        let hits = store.search("tags", None, None, &clauses).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "match-both");
    }

    // -- last with where_clauses --

    #[test]
    fn last_with_where() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "a",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "b",
                Some(serde_json::json!({"status": "closed"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "c",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        let items = store.last("tags", 10, None, &clauses).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].value, "a");
        assert_eq!(items[1].value, "c");
    }

    // -- random with where_clauses --

    #[test]
    fn random_with_where() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "active-1",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "closed-1",
                Some(serde_json::json!({"status": "closed"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "active-2",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        let items = store.random("tags", 10, None, &clauses).unwrap();
        assert_eq!(items.len(), 2);
        // All returned items should contain "active-"
        for item in &items {
            assert!(
                item.value.contains("active-"),
                "Expected active item, got: {}",
                item.value
            );
        }
    }

    // -- count with where_clauses --

    #[test]
    fn count_with_where() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "tags",
                "a",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "b",
                Some(serde_json::json!({"status": "closed"})),
                None,
            )
            .unwrap();
        store
            .push(
                "tags",
                "c",
                Some(serde_json::json!({"status": "active"})),
                None,
            )
            .unwrap();

        let clauses = vec![("status".to_string(), "active".to_string())];
        let result = store.count("tags", None, None, &clauses).unwrap();
        assert_eq!(result.matched, 2);
        assert_eq!(result.total, Some(3)); // where_clauses count as filtering
    }

    // -- search hits carry data field --

    #[test]
    fn search_hits_carry_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"status": "active"});
        store
            .push("tags", "item", Some(data.clone()), None)
            .unwrap();

        let hits = store.search("tags", Some("item"), None, &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].data, Some(data));
    }

    #[test]
    fn get_entries_by_id_carries_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"priority": "high"});
        store
            .push("tags", "item", Some(data.clone()), None)
            .unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(1)])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].data, Some(data));
    }

    // -- format_data_suffix --

    #[test]
    fn format_data_suffix_none() {
        assert_eq!(format_data_suffix(&None), "");
    }

    #[test]
    fn format_data_suffix_some() {
        let data = Some(serde_json::json!({"status": "active"}));
        let suffix = format_data_suffix(&data);
        assert!(suffix.starts_with(' '));
        // Should be compact JSON (no pretty printing)
        assert!(!suffix.contains('\n'));
        assert!(suffix.contains("\"status\""));
        assert!(suffix.contains("\"active\""));
    }

    // -- format_value includes data --

    #[test]
    fn format_value_includes_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"k": "v"});
        store.push("tags", "item", Some(data), None).unwrap();

        let formatted = format_value(store.get("tags").unwrap());
        assert!(formatted.contains("{\"k\":\"v\"}"));
    }

    #[test]
    fn format_value_no_data_unchanged() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "plain-item", None, None).unwrap();

        let formatted = format_value(store.get("tags").unwrap());
        assert!(formatted.contains("plain-item"));
        // Should NOT have trailing JSON
        assert!(!formatted.contains('{'));
    }

    // -- last output includes data suffix --

    #[test]
    fn last_output_includes_data() {
        let (mut store, _dir) = setup_store(test_schema());
        let data = serde_json::json!({"x": 1});
        store.push("tags", "item", Some(data), None).unwrap();

        let items = store.last("tags", 1, None, &[]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].data, Some(serde_json::json!({"x": 1})));
    }

    // -- Hash ID tests --

    #[test]
    fn push_returns_push_result_with_hash_and_id() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();
        assert_eq!(result.id, 1);
        assert!(!result.hash.is_empty());
        // Hash should be 5-6 base58 chars
        assert!(result.hash.len() >= 4);
        assert!(result.hash.len() <= 8);
    }

    #[test]
    fn hash_is_stable_same_inputs() {
        // generate_entry_hash is deterministic
        let h1 = generate_entry_hash("tags", "2026-05-08T00:00:00+00:00", 1);
        let h2 = generate_entry_hash("tags", "2026-05-08T00:00:00+00:00", 1);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_is_unique_different_inputs() {
        let h1 = generate_entry_hash("tags", "2026-05-08T00:00:00+00:00", 1);
        let h2 = generate_entry_hash("tags", "2026-05-08T00:00:00+00:00", 2);
        let h3 = generate_entry_hash("other", "2026-05-08T00:00:00+00:00", 1);
        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn backfill_old_entries_get_hashes_on_load() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Write old-format data with no hash field
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-05-08T00:00:00+00:00",
            "tags": {
                "items": [
                    {"id": 1, "value": "alpha", "ts": "2026-05-08T00:00:00+00:00"},
                    {"id": 2, "value": "beta", "ts": "2026-05-08T00:01:00+00:00"}
                ]
            }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();
        match store.data.entries.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert!(!items[0].hash.is_empty(), "hash should be back-filled");
                assert!(!items[1].hash.is_empty(), "hash should be back-filled");
                // Hashes should differ
                assert_ne!(items[0].hash, items[1].hash);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn get_by_numeric_id_still_works() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(result.id)])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "alpha");
    }

    #[test]
    fn get_by_hash_works() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Hash(result.hash.clone())])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "alpha");
        assert_eq!(hits[0].hash, result.hash);
    }

    #[test]
    fn get_by_hash_prefix_works() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        // Use first 3 chars as prefix
        let prefix = &result.hash[..3];
        let hits = store
            .get_entries_by_id("tags", &[IdRef::Hash(prefix.to_string())])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "alpha");
    }

    #[test]
    fn get_by_mixed_id_types() {
        let (mut store, _dir) = setup_store(test_schema());
        let r1 = store.push("tags", "alpha", None, None).unwrap();
        let r2 = store.push("tags", "beta", None, None).unwrap();

        let hits = store
            .get_entries_by_id(
                "tags",
                &[IdRef::Numeric(r1.id), IdRef::Hash(r2.hash.clone())],
            )
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn remove_by_hash_works() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        let r2 = store.push("tags", "beta", None, None).unwrap();
        store.push("tags", "gamma", None, None).unwrap();

        let result = store
            .remove("tags", None, Some(&IdRef::Hash(r2.hash.clone())), false)
            .unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "beta");

        // Verify beta is gone
        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].value, "alpha");
                assert_eq!(items[1].value, "gamma");
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn remove_by_ambiguous_hash_prefix_errors() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();

        // Manually set both entries to share a hash prefix.
        match store.data.entries.get_mut("tags").unwrap() {
            DataValue::List { items, .. } => {
                items[0].hash = "ABCxyz1".to_string();
                items[1].hash = "ABCxyz2".to_string();
            }
            _ => panic!("Expected list"),
        }

        // Removing by the shared prefix "ABC" should return an ambiguity error.
        let result = store.remove("tags", None, Some(&IdRef::Hash("ABC".to_string())), false);
        assert!(result.is_err(), "expected ambiguity error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ambiguous"),
            "error should mention ambiguity, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("matches 2 entries"),
            "error should report match count, got: {}",
            err_msg
        );
    }

    #[test]
    fn hash_appears_in_last_output() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        let items = store.last("tags", 1, None, &[]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].hash, result.hash);
    }

    #[test]
    fn hash_appears_in_search_output() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        let hits = store.search("tags", Some("alpha"), None, &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].hash, result.hash);
    }

    #[test]
    fn hash_stored_on_entry_structs() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].hash, result.hash);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn hash_stored_on_history_entry() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries[0].hash, result.hash);
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn hash_persists_through_save_load() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        let hash;
        {
            let mut store = KvStore::load(&schema_path, &data_path).unwrap();
            let result = store.push("tags", "alpha", None, None).unwrap();
            hash = result.hash;
            store.save().unwrap();
        }

        // Reload and verify hash persisted
        let store2 = KvStore::load(&schema_path, &data_path).unwrap();
        match store2.data.entries.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].hash, hash);
            }
            _ => panic!("Expected list"),
        }
    }

    // -- Per-entry memory tests --

    #[test]
    fn push_with_memory_stores_on_entry() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("tags", "linked-item", None, Some("kn-abc123".to_string()))
            .unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].memory, Some("kn-abc123".to_string()));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn push_without_memory_has_none() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "plain-item", None, None).unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert!(items[0].memory.is_none());
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn push_history_with_memory() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push(
                "flavor_history",
                "bergamot",
                None,
                Some("kn-hist123".to_string()),
            )
            .unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries[0].memory, Some("kn-hist123".to_string()));
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn set_entry_memory_by_numeric_id() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        store
            .set_entry_memory(
                "tags",
                &IdRef::Numeric(result.id),
                Some("kn-set1".to_string()),
            )
            .unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].memory, Some("kn-set1".to_string()));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn set_entry_memory_by_hash() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.push("tags", "alpha", None, None).unwrap();

        store
            .set_entry_memory(
                "tags",
                &IdRef::Hash(result.hash.clone()),
                Some("kn-hash1".to_string()),
            )
            .unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].memory, Some("kn-hash1".to_string()));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn set_entry_memory_not_found_errors() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();

        let result = store.set_entry_memory("tags", &IdRef::Numeric(999), Some("kn-x".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Entry not found"));
    }

    #[test]
    fn set_entry_memory_history_by_numeric_id() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store
            .push("flavor_history", "bergamot", None, None)
            .unwrap();

        store
            .set_entry_memory(
                "flavor_history",
                &IdRef::Numeric(result.id),
                Some("kn-hist-set".to_string()),
            )
            .unwrap();

        match store.get("flavor_history").unwrap() {
            DataValue::History { entries, .. } => {
                assert_eq!(entries[0].memory, Some("kn-hist-set".to_string()));
            }
            _ => panic!("Expected history"),
        }
    }

    #[test]
    fn per_entry_memory_in_search_hit() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("tags", "linked", None, Some("kn-search1".to_string()))
            .unwrap();
        store.push("tags", "plain", None, None).unwrap();

        let hits = store.search("tags", Some("linked"), None, &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory, Some("kn-search1".to_string()));

        let hits = store.search("tags", Some("plain"), None, &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].memory.is_none());
    }

    #[test]
    fn last_returns_search_hit_with_memory() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("tags", "with-mem", None, Some("kn-last1".to_string()))
            .unwrap();
        store.push("tags", "without-mem", None, None).unwrap();

        let hits = store.last("tags", 2, None, &[]).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].memory, Some("kn-last1".to_string()));
        assert!(hits[1].memory.is_none());
    }

    #[test]
    fn random_returns_search_hit_with_memory() {
        let (mut store, _dir) = setup_store(test_schema());
        store
            .push("tags", "only-item", None, Some("kn-rand1".to_string()))
            .unwrap();

        let hits = store.random("tags", 1, None, &[]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory, Some("kn-rand1".to_string()));
    }

    #[test]
    fn backward_compat_data_without_memory_field() {
        // Simulate old data without memory field on entries
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        // Write data file without memory field on entries
        let old_data = r#"{
            "_schema": "test",
            "_updated": "2026-01-01T00:00:00Z",
            "tags": {
                "items": [
                    {"id": 1, "hash": "abc", "value": "old-item", "ts": "2026-01-01T00:00:00Z"}
                ]
            }
        }"#;
        fs::write(&data_path, old_data).unwrap();

        let store = KvStore::load(&schema_path, &data_path).unwrap();
        match store.data.entries.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].value, "old-item");
                assert!(items[0].memory.is_none());
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn clear_entry_memory_with_empty_string() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store
            .push("tags", "alpha", None, Some("kn-clear1".to_string()))
            .unwrap();

        // Set memory to empty string should clear it
        store
            .set_entry_memory("tags", &IdRef::Numeric(result.id), Some("".to_string()))
            .unwrap();

        match store.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert!(items[0].memory.is_none());
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn get_entries_by_id_carries_memory() {
        let (mut store, _dir) = setup_store(test_schema());
        let r1 = store
            .push("tags", "with-mem", None, Some("kn-byid1".to_string()))
            .unwrap();
        store.push("tags", "without", None, None).unwrap();

        let hits = store
            .get_entries_by_id("tags", &[IdRef::Numeric(r1.id)])
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory, Some("kn-byid1".to_string()));
    }

    #[test]
    fn set_entry_memory_type_mismatch() {
        let (mut store, _dir) = setup_store(test_schema());
        store.inc("warmth", 1).unwrap();

        let result = store.set_entry_memory("warmth", &IdRef::Numeric(1), Some("kn-x".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    #[test]
    fn set_entry_memory_ambiguous_hash_prefix_errors() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha", None, None).unwrap();
        store.push("tags", "beta", None, None).unwrap();

        // Manually set both entries to share a hash prefix.
        match store.data.entries.get_mut("tags").unwrap() {
            DataValue::List { items, .. } => {
                items[0].hash = "XYZaaa1".to_string();
                items[1].hash = "XYZaaa2".to_string();
            }
            _ => panic!("Expected list"),
        }

        // Setting memory by the shared prefix "XYZ" should return an ambiguity error.
        let result = store.set_entry_memory(
            "tags",
            &IdRef::Hash("XYZ".to_string()),
            Some("kn-x".to_string()),
        );
        assert!(result.is_err(), "expected ambiguity error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ambiguous"),
            "error should mention ambiguity, got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("matches 2 entries"),
            "error should report match count, got: {}",
            err_msg
        );
    }

    #[test]
    fn entry_memory_persists_through_save_reload() {
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        let mut f = fs::File::create(&schema_path).unwrap();
        f.write_all(test_schema().as_bytes()).unwrap();

        {
            let mut store = KvStore::load(&schema_path, &data_path).unwrap();
            store
                .push("tags", "persistent", None, Some("kn-persist".to_string()))
                .unwrap();
            store.save().unwrap();
        }

        let store2 = KvStore::load(&schema_path, &data_path).unwrap();
        match store2.data.entries.get("tags").unwrap() {
            DataValue::List { items, .. } => {
                assert_eq!(items[0].memory, Some("kn-persist".to_string()));
            }
            _ => panic!("Expected list"),
        }
    }

    // -- add_key_to_schema --

    #[test]
    fn add_key_to_schema_history() {
        let (mut store, _dir) = setup_store(test_schema());
        assert!(!store.schema.keys.contains_key("puns"));

        store.add_key_to_schema("puns", "history", None).unwrap();

        // Key should now exist in the in-memory schema
        let def = store.schema.keys.get("puns").unwrap();
        assert_eq!(def.value_type, ValueType::History);
        assert_eq!(def.max_entries, None);

        // Push should now work on the new key
        store.push("puns", "the joke", None, None).unwrap();
        let last = store.last("puns", 1, None, &[]).unwrap();
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].value, "the joke");
    }

    #[test]
    fn add_key_to_schema_list() {
        let (mut store, _dir) = setup_store(test_schema());
        assert!(!store.schema.keys.contains_key("items"));

        store.add_key_to_schema("items", "list", None).unwrap();

        let def = store.schema.keys.get("items").unwrap();
        assert_eq!(def.value_type, ValueType::List);

        store.push("items", "apple", None, None).unwrap();
        let last = store.last("items", 1, None, &[]).unwrap();
        assert_eq!(last[0].value, "apple");
    }

    #[test]
    fn add_key_to_schema_with_max_entries() {
        let (mut store, _dir) = setup_store(test_schema());

        store
            .add_key_to_schema("puns", "history", Some(500))
            .unwrap();

        let def = store.schema.keys.get("puns").unwrap();
        assert_eq!(def.value_type, ValueType::History);
        assert_eq!(def.max_entries, Some(500));
    }

    #[test]
    fn add_key_to_schema_existing_key_is_noop() {
        let (mut store, _dir) = setup_store(test_schema());

        // "tags" already exists as a list in the test schema
        assert!(store.schema.keys.contains_key("tags"));
        let original_type = store.schema.keys["tags"].value_type;

        // Should succeed silently without changing anything
        store.add_key_to_schema("tags", "history", None).unwrap();

        // Type should be unchanged
        assert_eq!(store.schema.keys["tags"].value_type, original_type);
    }

    // Type validation for --create is now enforced by clap's ValueEnum
    // (CreateType), so there is no add_key_to_schema_invalid_type_rejected
    // test -- invalid types cannot reach add_key_to_schema.

    #[test]
    fn add_key_to_schema_dotted_name_rejected() {
        let (mut store, _dir) = setup_store(test_schema());

        let err = store
            .add_key_to_schema("my.key", "history", None)
            .unwrap_err();
        assert!(err.to_string().contains("cannot contain dots"));
    }

    #[test]
    fn add_key_to_schema_special_chars_rejected() {
        let (mut store, _dir) = setup_store(test_schema());

        let err = store
            .add_key_to_schema("my key!", "history", None)
            .unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn add_key_to_schema_empty_name_rejected() {
        let (mut store, _dir) = setup_store(test_schema());

        let err = store.add_key_to_schema("", "history", None).unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    fn add_key_to_schema_reparsed_correctly() {
        let (mut store, _dir) = setup_store(test_schema());
        let original_key_count = store.schema.keys.len();

        store
            .add_key_to_schema("new_key", "history", Some(50))
            .unwrap();

        // Should have exactly one more key
        assert_eq!(store.schema.keys.len(), original_key_count + 1);

        // Reload from disk to verify file was written correctly
        let store2 = KvStore::load(&store.schema_path, &store.data_path).unwrap();
        assert_eq!(store2.schema.keys.len(), original_key_count + 1);
        let def = store2.schema.keys.get("new_key").unwrap();
        assert_eq!(def.value_type, ValueType::History);
        assert_eq!(def.max_entries, Some(50));
    }

    #[test]
    fn add_key_to_schema_preserves_existing_content() {
        let (mut store, _dir) = setup_store(test_schema());

        // Push some data to an existing key first
        store.push("tags", "hello", None, None).unwrap();
        store.save().unwrap();

        // Now add a new key to schema
        store.add_key_to_schema("jokes", "list", None).unwrap();

        // Original keys should all still be present and correct
        assert_eq!(store.schema.keys["warmth"].value_type, ValueType::Counter);
        assert_eq!(
            store.schema.keys["flavor_history"].value_type,
            ValueType::History
        );
        assert_eq!(store.schema.keys["tags"].value_type, ValueType::List);
        assert_eq!(store.schema.keys["tensor"].value_type, ValueType::State);
        assert_eq!(
            store.schema.keys["current_mood"].value_type,
            ValueType::String
        );

        // New key should be there too
        assert_eq!(store.schema.keys["jokes"].value_type, ValueType::List);
    }

    #[test]
    fn add_key_hyphen_and_underscore_accepted() {
        let (mut store, _dir) = setup_store(test_schema());

        store
            .add_key_to_schema("my-key_v2", "history", None)
            .unwrap();
        assert!(store.schema.keys.contains_key("my-key_v2"));
    }

    #[test]
    fn add_key_to_schema_name_too_long_rejected() {
        let (mut store, _dir) = setup_store(test_schema());
        let long_name = "a".repeat(129);

        let err = store
            .add_key_to_schema(&long_name, "history", None)
            .unwrap_err();
        assert!(err.to_string().contains("key name too long"));
        assert!(err.to_string().contains("129 chars"));
        assert!(err.to_string().contains("max 128"));
    }

    #[test]
    fn add_key_to_schema_max_length_accepted() {
        let (mut store, _dir) = setup_store(test_schema());
        let max_name = "a".repeat(128);

        store.add_key_to_schema(&max_name, "history", None).unwrap();
        assert!(store.schema.keys.contains_key(max_name.as_str()));
    }

    #[test]
    fn add_key_to_schema_without_trailing_newline() {
        // S2: Schema file without trailing newline should still produce valid TOML
        // after add_key_to_schema appends a new key block.
        let schema_no_newline = "[keys.existing]\ntype = \"history\"";
        let dir = TempDir::new().unwrap();
        let schema_path = dir.path().join("test.schema.toml");
        let data_path = dir.path().join("test.data.json");

        // Write schema WITHOUT trailing newline
        fs::write(&schema_path, schema_no_newline).unwrap();

        let mut store = KvStore::load(&schema_path, &data_path).unwrap();
        assert!(store.schema.keys.contains_key("existing"));

        // Add a new key -- should not corrupt the file
        store.add_key_to_schema("newkey", "list", None).unwrap();

        // Both keys should be present after re-parse
        assert!(store.schema.keys.contains_key("existing"));
        assert!(store.schema.keys.contains_key("newkey"));
        assert_eq!(store.schema.keys["existing"].value_type, ValueType::History);
        assert_eq!(store.schema.keys["newkey"].value_type, ValueType::List);

        // Verify file on disk is valid TOML by reloading
        let store2 = KvStore::load(&schema_path, &data_path).unwrap();
        assert!(store2.schema.keys.contains_key("existing"));
        assert!(store2.schema.keys.contains_key("newkey"));
    }
}
