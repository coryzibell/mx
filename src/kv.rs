//! Lightweight local KV store for fast agent state.
//!
//! Backed by a TOML schema file and a JSON data file. All writes are atomic
//! (serialize to tmp, fsync, rename). No networking, no database.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

pub const EXIT_OK: i32 = 0;
pub const EXIT_KEY_NOT_FOUND: i32 = 1;
pub const EXIT_TYPE_MISMATCH: i32 = 2;
pub const EXIT_SCHEMA_MISSING: i32 = 3;

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
                                value: s,
                                ts: String::new(),
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
    pub value: String,
    pub ts: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ListEntry {
    pub id: u64,
    pub value: String,
    pub ts: String,
}

/// Remove result for the CLI layer to format output.
#[derive(Debug)]
pub struct RemoveResult {
    pub removed: Vec<String>,
}

/// Search result entry.
#[derive(Debug)]
pub struct SearchHit {
    pub id: u64,
    pub value: String,
    pub ts: String,
}

/// Count result.
#[derive(Debug)]
pub struct CountResult {
    pub total: usize,
    pub latest_ts: Option<String>,
}

// ---------------------------------------------------------------------------
// KV Store
// ---------------------------------------------------------------------------

pub struct KvStore {
    pub schema: Schema,
    pub data: DataFile,
    pub data_path: PathBuf,
}

impl KvStore {
    /// Load schema and data from the given paths. Creates data file with defaults
    /// if it doesn't exist.
    pub fn load(schema_path: &Path, data_path: &Path) -> Result<Self> {
        let schema_str = fs::read_to_string(schema_path)
            .with_context(|| format!("Failed to read schema: {}", schema_path.display()))?;
        let schema: Schema = toml::from_str(&schema_str)
            .with_context(|| format!("Failed to parse schema: {}", schema_path.display()))?;

        let data = if data_path.exists() {
            let data_str = fs::read_to_string(data_path)
                .with_context(|| format!("Failed to read data: {}", data_path.display()))?;
            serde_json::from_str(&data_str)
                .with_context(|| format!("Failed to parse data: {}", data_path.display()))?
        } else {
            DataFile::default()
        };

        Ok(KvStore {
            schema,
            data,
            data_path: data_path.to_path_buf(),
        })
    }

    /// Load from environment variables. Resolves {agent} placeholder.
    pub fn from_env() -> Result<Self> {
        let agent = std::env::var("MX_CURRENT_AGENT")
            .with_context(|| "MX_CURRENT_AGENT environment variable is required")?;

        let default_schema = Self::default_schema_path(&agent)
            .with_context(|| "Could not determine home directory for schema path")?;
        let default_data = Self::default_data_path(&agent)
            .with_context(|| "Could not determine home directory for data path")?;

        let schema_path = std::env::var("MX_KV_SCHEMA")
            .map(|s| PathBuf::from(s.replace("{agent}", &agent)))
            .unwrap_or(default_schema);

        let data_path = std::env::var("MX_KV_DATA")
            .map(|s| PathBuf::from(s.replace("{agent}", &agent)))
            .unwrap_or(default_data);

        let mut store = Self::load(&schema_path, &data_path)?;

        // Populate _schema field from agent name if empty (SHOULD-FIX 5)
        if store.data.schema_id.is_empty() {
            store.data.schema_id = agent.clone();
        }

        Ok(store)
    }

    fn default_schema_path(agent: &str) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".crewu")
            .join("kv")
            .join(format!("{}.schema.toml", agent)))
    }

    fn default_data_path(agent: &str) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .context("Could not determine home directory")?
            .join(".crewu")
            .join("kv")
            .join(format!("{}.data.json", agent)))
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
    pub fn push(&mut self, key: &str, value: &str) -> Result<(), KvError> {
        self.push_with_ts(key, value, Utc::now())
    }

    /// Push with an explicit timestamp (used by tests).
    pub fn push_with_ts(
        &mut self,
        key: &str,
        value: &str,
        ts: DateTime<Utc>,
    ) -> Result<(), KvError> {
        let def = self.key_def(key)?.clone();

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
                        entries.insert(
                            0,
                            HistoryEntry {
                                id: next_id,
                                value: value.to_string(),
                                ts: ts.to_rfc3339(),
                            },
                        );
                        // Drop oldest at max_entries
                        if let Some(max) = def.max_entries {
                            entries.truncate(max);
                        }
                    }
                    _ => {
                        return Err(KvError::Other(anyhow::anyhow!(
                            "Data corruption: key '{}' has wrong runtime type",
                            key
                        )));
                    }
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
                        items.push(ListEntry {
                            id: next_id,
                            value: value.to_string(),
                            ts: ts.to_rfc3339(),
                        });
                        // Drop oldest at max_entries — single drain instead of O(n^2) remove loop
                        if let Some(max) = def.max_entries
                            && items.len() > max
                        {
                            items.drain(0..items.len() - max);
                        }
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
                    expected: "history or list".to_string(),
                    got: def.value_type.to_string(),
                });
            }
        }

        Ok(())
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

    /// Get the last N entries from a history or list, formatted with IDs and timestamps.
    pub fn last(&self, key: &str, count: usize) -> Result<Vec<String>, KvError> {
        let def = self.key_def(key)?;

        match def.value_type {
            ValueType::History => match self.data.entries.get(key) {
                Some(DataValue::History { entries, .. }) => Ok(entries
                    .iter()
                    .take(count)
                    .map(|e| format!("{}: {} ({})", e.id, e.value, e.ts))
                    .collect()),
                _ => Ok(vec![]),
            },
            ValueType::List => match self.data.entries.get(key) {
                Some(DataValue::List { items, .. }) => {
                    let start = items.len().saturating_sub(count);
                    Ok(items[start..]
                        .iter()
                        .map(|e| {
                            if e.ts.is_empty() {
                                format!("{}: {}", e.id, e.value)
                            } else {
                                format!("{}: {} ({})", e.id, e.value, e.ts)
                            }
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

    /// Remove entries from a list or history by value substring or by ID.
    ///
    /// - `by_id`: if Some, remove the entry with that ID (ignores `value` and `all`).
    /// - `value`: substring match (case-insensitive).
    /// - `all`: if true, remove all matches; otherwise remove only the first match.
    pub fn remove(
        &mut self,
        key: &str,
        value: Option<&str>,
        by_id: Option<u64>,
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
                if let Some(id) = by_id {
                    if let Some(pos) = entries.iter().position(|e| e.id == id) {
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
                if let Some(id) = by_id {
                    if let Some(pos) = items.iter().position(|e| e.id == id) {
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

    /// Search entries in a list or history by case-insensitive substring.
    pub fn search(&self, key: &str, query: &str) -> Result<Vec<SearchHit>, KvError> {
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

        let query_lower = query.to_lowercase();
        let mut hits = Vec::new();

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => {
                for e in entries {
                    if e.value.to_lowercase().contains(&query_lower) {
                        hits.push(SearchHit {
                            id: e.id,
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                        });
                    }
                }
            }
            Some(DataValue::List { items, .. }) => {
                for e in items {
                    if e.value.to_lowercase().contains(&query_lower) {
                        hits.push(SearchHit {
                            id: e.id,
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                        });
                    }
                }
            }
            _ => {}
        }

        Ok(hits)
    }

    /// Count entries in a list or history, optionally filtered by substring.
    pub fn count(&self, key: &str, value: Option<&str>) -> Result<CountResult, KvError> {
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
        let mut total = 0usize;
        let mut latest_ts: Option<String> = None;

        match self.data.entries.get(key) {
            Some(DataValue::History { entries, .. }) => {
                for e in entries {
                    let matches = match &query_lower {
                        Some(q) => e.value.to_lowercase().contains(q),
                        None => true,
                    };
                    if matches {
                        total += 1;
                        if latest_ts.is_none() || e.ts > *latest_ts.as_ref().unwrap() {
                            latest_ts = Some(e.ts.clone());
                        }
                    }
                }
            }
            Some(DataValue::List { items, .. }) => {
                for e in items {
                    let matches = match &query_lower {
                        Some(q) => e.value.to_lowercase().contains(q),
                        None => true,
                    };
                    if matches {
                        total += 1;
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

        Ok(CountResult { total, latest_ts })
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
// Display helpers (for CLI output)
// ---------------------------------------------------------------------------

/// Format a DataValue for human-readable CLI output.
pub fn format_value(value: &DataValue) -> String {
    match value {
        DataValue::Counter { value } => value.to_string(),
        DataValue::String { value } => value.clone(),
        DataValue::History { entries, .. } => entries
            .iter()
            .map(|e| format!("{}: {} ({})", e.id, e.value, e.ts))
            .collect::<Vec<_>>()
            .join("\n"),
        DataValue::State { fields, .. } => {
            serde_json::to_string_pretty(fields).unwrap_or_else(|_| "{}".to_string())
        }
        DataValue::List { items, .. } => items
            .iter()
            .map(|e| {
                if e.ts.is_empty() {
                    format!("{}: {}", e.id, e.value)
                } else {
                    format!("{}: {} ({})", e.id, e.value, e.ts)
                }
            })
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
        store.push("flavor_history", "bergamot").unwrap();
        store.push("flavor_history", "lapsang").unwrap();

        let last = store.last("flavor_history", 1).unwrap();
        assert_eq!(last.len(), 1);
        // last now includes id and ts
        assert!(last[0].contains("lapsang"));

        let last2 = store.last("flavor_history", 2).unwrap();
        assert_eq!(last2.len(), 2);
    }

    #[test]
    fn history_max_entries_overflow() {
        let (mut store, _dir) = setup_store(test_schema());
        // max_entries = 3
        store.push("flavor_history", "a").unwrap();
        store.push("flavor_history", "b").unwrap();
        store.push("flavor_history", "c").unwrap();
        store.push("flavor_history", "d").unwrap();

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
            .push_with_ts("flavor_history", "old_one", old_time)
            .unwrap();
        store
            .push_with_ts("flavor_history", "new_one", new_time)
            .unwrap();

        let results = store.since("flavor_history", "1h").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "new_one");
    }

    // -- List operations --

    #[test]
    fn list_push_pop_last() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha").unwrap();
        store.push("tags", "beta").unwrap();
        store.push("tags", "gamma").unwrap();

        let last = store.last("tags", 2).unwrap();
        assert_eq!(last.len(), 2);
        assert!(last[0].contains("beta"));
        assert!(last[1].contains("gamma"));

        let popped = store.pop("tags").unwrap();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().value, "gamma");
    }

    #[test]
    fn list_max_entries_overflow() {
        let (mut store, _dir) = setup_store(test_schema());
        // max_entries = 5
        for i in 0..8 {
            store.push("tags", &format!("item_{}", i)).unwrap();
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
        let result = store.push("warmth", "value");
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
        store.push("flavor_history", "test").unwrap();
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
        store.push("flavor_history", "earl grey").unwrap();
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
        store.push_with_ts("tags", "focus", ts).unwrap();
        store.push_with_ts("tags", "rust", ts).unwrap();

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
            .push_with_ts("flavor_history", "bergamot", ts)
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
        let last = store.last("flavor_history", 5).unwrap();
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
        store.push("tags", "alpha").unwrap();
        store.push("tags", "beta").unwrap();
        store.push("tags", "alpha-2").unwrap();

        let result = store.remove("tags", Some("alpha"), None, false).unwrap();
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
        store.push("tags", "alpha").unwrap();
        store.push("tags", "beta").unwrap();
        store.push("tags", "alpha-2").unwrap();

        let result = store.remove("tags", Some("alpha"), None, true).unwrap();
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
        store.push("tags", "alpha").unwrap();
        store.push("tags", "beta").unwrap();
        store.push("tags", "gamma").unwrap();

        // Get the ID of "beta"
        let beta_id = match store.get("tags").unwrap() {
            DataValue::List { items, .. } => items[1].id,
            _ => panic!("Expected list"),
        };

        let result = store.remove("tags", None, Some(beta_id), false).unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "beta");
    }

    #[test]
    fn remove_history_by_value() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();
        store.push("flavor_history", "lapsang").unwrap();
        store.push("flavor_history", "bergamot vanilla").unwrap();

        let result = store
            .remove("flavor_history", Some("bergamot"), None, false)
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
        store.push("tags", "Alpha").unwrap();
        store.push("tags", "beta").unwrap();

        let result = store.remove("tags", Some("alpha"), None, false).unwrap();
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0], "Alpha");
    }

    #[test]
    fn remove_type_mismatch() {
        let (mut store, _dir) = setup_store(test_schema());
        let result = store.remove("warmth", Some("x"), None, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- Search operations --

    #[test]
    fn search_list() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "rust-lang").unwrap();
        store.push("tags", "focus").unwrap();
        store.push("tags", "rust-tools").unwrap();

        let hits = store.search("tags", "rust").unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].value, "rust-lang");
        assert_eq!(hits[1].value, "rust-tools");
    }

    #[test]
    fn search_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();
        store.push("flavor_history", "lapsang").unwrap();
        store.push("flavor_history", "bergamot vanilla").unwrap();

        let hits = store.search("flavor_history", "bergamot").unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_case_insensitive() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "Rust").unwrap();
        store.push("tags", "RUST-tools").unwrap();
        store.push("tags", "python").unwrap();

        let hits = store.search("tags", "rust").unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_no_matches() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "alpha").unwrap();

        let hits = store.search("tags", "zzz").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_type_mismatch() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.search("warmth", "x");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- Count operations --

    #[test]
    fn count_list_total() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "a").unwrap();
        store.push("tags", "b").unwrap();
        store.push("tags", "c").unwrap();

        let result = store.count("tags", None).unwrap();
        assert_eq!(result.total, 3);
    }

    #[test]
    fn count_list_filtered() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("tags", "rust-lang").unwrap();
        store.push("tags", "focus").unwrap();
        store.push("tags", "rust-tools").unwrap();

        let result = store.count("tags", Some("rust")).unwrap();
        assert_eq!(result.total, 2);
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
            .push_with_ts("flavor_history", "bergamot", ts1)
            .unwrap();
        store
            .push_with_ts("flavor_history", "bergamot vanilla", ts2)
            .unwrap();
        store
            .push_with_ts("flavor_history", "lapsang", ts1)
            .unwrap();

        let result = store.count("flavor_history", Some("bergamot")).unwrap();
        assert_eq!(result.total, 2);
        assert!(result.latest_ts.is_some());
        assert!(result.latest_ts.unwrap().contains("2026-04-22"));
    }

    #[test]
    fn count_empty() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.count("tags", None).unwrap();
        assert_eq!(result.total, 0);
        assert!(result.latest_ts.is_none());
    }

    #[test]
    fn count_type_mismatch() {
        let (store, _dir) = setup_store(test_schema());
        let result = store.count("warmth", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Type mismatch"));
    }

    // -- ID assignment --

    #[test]
    fn history_ids_auto_increment() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "a").unwrap();
        store.push("flavor_history", "b").unwrap();
        store.push("flavor_history", "c").unwrap();

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
        store.push("tags", "a").unwrap();
        store.push("tags", "b").unwrap();
        store.push("tags", "c").unwrap();

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
        store.push("tags", "a").unwrap();
        store.push("tags", "b").unwrap();
        store.push("tags", "c").unwrap();

        // Remove "b"
        store.remove("tags", Some("b"), None, false).unwrap();

        // Push "d" — should get id=4, not reuse id=2
        store.push("tags", "d").unwrap();

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
        store.push_with_ts("tags", "focus", ts).unwrap();

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
        store.push_with_ts("tags", "focus", ts).unwrap();
        store.push_with_ts("tags", "rust", ts).unwrap();

        let output = format_value(store.get("tags").unwrap());
        assert!(output.contains("1: focus"));
        assert!(output.contains("2: rust"));
        assert!(output.contains("2026-04-22"));
    }

    #[test]
    fn format_value_history_shows_ids() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();

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
        store.push_with_ts("tags", "dsi-panel", ts).unwrap();
        store.push_with_ts("tags", "anytype", ts).unwrap();

        let compact = store.dump_compact();
        assert!(compact.contains("tags=[dsi-panel@14:15,anytype@14:15]"));
    }

    use chrono::Datelike as _;

    // -- Memory pointer operations --

    #[test]
    fn set_get_memory_on_history() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();

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
        store.push("tags", "alpha").unwrap();

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
        store.push("flavor_history", "bergamot").unwrap();

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
        store.push("tags", "alpha").unwrap();

        store
            .set_memory("tags", Some("kn-abc123".to_string()))
            .unwrap();
        store.set_memory("tags", None).unwrap();
        assert_eq!(store.get_memory("tags").unwrap(), None);
    }

    #[test]
    fn memory_not_serialized_when_none() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();
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
            .push_with_ts("flavor_history", "bergamot", ts)
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
            .push_with_ts("flavor_history", "bergamot", ts)
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
        store.push("flavor_history", "bergamot").unwrap();
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();

        // Push another entry
        store.push("flavor_history", "lapsang").unwrap();

        // Memory pointer should still be set
        assert_eq!(
            store.get_memory("flavor_history").unwrap(),
            Some("kn-abc123")
        );
    }

    #[test]
    fn memory_survives_reset() {
        let (mut store, _dir) = setup_store(test_schema());
        store.push("flavor_history", "bergamot").unwrap();
        store
            .set_memory("flavor_history", Some("kn-abc123".to_string()))
            .unwrap();

        // Reset clears data to default — memory pointer should be cleared too
        store.reset("flavor_history").unwrap();

        // After reset, memory is gone (default_value has memory: None)
        assert_eq!(store.get_memory("flavor_history").unwrap(), None);
    }
}
