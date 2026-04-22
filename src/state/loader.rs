//! Schema loading utilities
//!
//! Functions for locating and loading state schema files from disk.

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

use super::schema::{DynamicState, StateSchema};

/// Load schema from file
pub fn load_schema(path: &Path) -> Result<StateSchema> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read schema file: {:?}", path))?;

    serde_json::from_str(&content).with_context(|| format!("Failed to parse schema: {:?}", path))
}

/// Load the default emotional state schema
///
/// Schema lookup order:
/// 1. MX_STATE_SCHEMA environment variable (explicit path)
/// 2. MX_CURRENT_AGENT environment variable (looks for ~/.{agent}/schemas/state.json)
/// 3. Standard fallback locations ($MX_HOME/schemas/emotional-state.json, /etc/mx/schemas/emotional-state.json)
pub fn load_default_schema() -> Result<StateSchema> {
    // 1. Check MX_STATE_SCHEMA environment variable
    if let Ok(schema_path) = std::env::var("MX_STATE_SCHEMA") {
        let path = std::path::PathBuf::from(&schema_path);
        if path.exists() {
            return load_schema(&path);
        } else {
            bail!(
                "MX_STATE_SCHEMA points to non-existent file: {}",
                schema_path
            );
        }
    }

    // 2. Check MX_CURRENT_AGENT environment variable
    if let Ok(agent) = std::env::var("MX_CURRENT_AGENT")
        && let Some(home) = dirs::home_dir()
    {
        let agent_schema = home.join(format!(".{}/schemas/state.json", agent));
        if agent_schema.exists() {
            return load_schema(&agent_schema);
        }
    }

    // 3. Try standard locations
    let locations = [
        Some(crate::paths::schemas_dir().join("emotional-state.json")),
        Some(std::path::PathBuf::from(
            "/etc/mx/schemas/emotional-state.json",
        )),
    ];

    for loc in locations.into_iter().flatten() {
        if loc.exists() {
            return load_schema(&loc);
        }
    }

    bail!(
        "Could not find state schema. Tried:\n\
         - MX_STATE_SCHEMA environment variable\n\
         - MX_CURRENT_AGENT environment variable (looks for ~/.{{agent}}/schemas/state.json)\n\
         - {}/emotional-state.json\n\
         - /etc/mx/schemas/emotional-state.json",
        crate::paths::schemas_dir().display()
    )
}

/// Parse a wake preference line and convert to DynamicState
/// Handles both old format (Wake Preference: soft) and new stele format
pub fn parse_wake_preference_dynamic(line: &str, schema: &StateSchema) -> Result<DynamicState> {
    let trimmed = line.trim();

    // Check for stele format first (starts with @state or schema-specific header)
    if trimmed.starts_with(&schema.stele.header) {
        return DynamicState::decode_stele(trimmed, schema);
    }

    // Check for old format: "Wake Preference: mode" or just "mode"
    let mode = if let Some(stripped) = trimmed.strip_prefix("Wake Preference:") {
        stripped.trim()
    } else if let Some(stripped) = trimmed.strip_prefix("Wake State:") {
        stripped.trim()
    } else {
        trimmed
    };

    // Map mode to state
    DynamicState::from_mode(mode, schema)
}
