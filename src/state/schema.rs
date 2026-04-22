//! Modern schema-driven state system
//!
//! Contains the dynamic, schema-agnostic state types and their encoding/decoding logic.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Schema-agnostic state value - can be float, enum, or nested structure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StateValue {
    Float(f32),
    Enum(String),
    Nested(HashMap<String, StateValue>),
}

/// Schema-agnostic dynamic state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicState {
    pub schema_id: String,
    pub values: HashMap<String, StateValue>,
}
impl DynamicState {
    /// Encode to stele format using provided schema
    pub fn encode_stele(&self, schema: &StateSchema) -> String {
        let s = &schema.stele;
        let mut parts = vec![format!("{}:{}", s.header, schema.name)];

        // Get dimension names in sorted order for deterministic output
        let mut dim_names: Vec<_> = schema.dimensions.keys().collect();
        dim_names.sort();

        for dim_name in dim_names {
            if let Some(dim_def) = schema.dimensions.get(dim_name.as_str())
                && let Some(value) = self.values.get(dim_name.as_str())
            {
                Self::encode_dimension(
                    &mut parts,
                    dim_name,
                    dim_def,
                    value,
                    &s.symbols,
                    &s.modality_values,
                    &s.separator,
                    &s.nested_separator,
                    "",
                );
            }
        }

        parts.join(&s.separator)
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::only_used_in_recursion)]
    fn encode_dimension(
        parts: &mut Vec<String>,
        name: &str,
        definition: &Dimension,
        value: &StateValue,
        symbols: &HashMap<String, String>,
        modality_values: &HashMap<String, String>,
        separator: &str,
        nested_separator: &str,
        prefix: &str,
    ) {
        let symbol = symbols.get(name).map(|s| s.as_str()).unwrap_or(name);

        match (definition, value) {
            (Dimension::Float { .. }, StateValue::Float(v)) => {
                parts.push(format!("{}{}{}", prefix, symbol, v));
            }
            (Dimension::Enum { .. }, StateValue::Enum(v)) => {
                // Check if there's a symbol mapping for this enum value
                let value_sym = modality_values.get(v).map(|s| s.as_str()).unwrap_or(v);
                parts.push(format!("{}{}{}", prefix, symbol, value_sym));
            }
            (Dimension::Nested { dimensions, .. }, StateValue::Nested(nested_values)) => {
                // For nested dimensions, encode each sub-dimension with prefix
                let new_prefix = if prefix.is_empty() {
                    format!("{}{}", symbol, nested_separator)
                } else {
                    format!("{}{}{}", prefix, symbol, nested_separator)
                };

                // Get nested dimension names in sorted order
                let mut nested_names: Vec<_> = dimensions.keys().collect();
                nested_names.sort();

                for nested_name in nested_names {
                    if let Some(nested_def) = dimensions.get(nested_name.as_str())
                        && let Some(nested_value) = nested_values.get(nested_name.as_str())
                    {
                        Self::encode_dimension(
                            parts,
                            nested_name,
                            nested_def,
                            nested_value,
                            symbols,
                            modality_values,
                            separator,
                            nested_separator,
                            &new_prefix,
                        );
                    }
                }
            }
            _ => {
                // Type mismatch - skip
            }
        }
    }

    /// Decode from stele format into DynamicState
    pub fn decode_stele(stele: &str, schema: &StateSchema) -> Result<Self> {
        let s = &schema.stele;
        let sep = &s.separator;
        let nsep = &s.nested_separator;

        // Build symbol maps
        let mut top_level_sym_to_name: HashMap<&str, &str> = HashMap::new();
        let mut nested_parent_syms: std::collections::HashSet<&str> =
            std::collections::HashSet::new();

        for (name, dim) in &schema.dimensions {
            if let Some(sym) = s.symbols.get(name.as_str()) {
                top_level_sym_to_name.insert(sym.as_str(), name.as_str());
                // Track which symbols have nested dimensions
                if matches!(dim, Dimension::Nested { .. }) {
                    nested_parent_syms.insert(sym.as_str());
                }
            }
        }

        // Build reverse modality map
        let mut rev_modality: HashMap<&str, &str> = HashMap::new();
        for (name, sym) in &s.modality_values {
            rev_modality.insert(sym.as_str(), name.as_str());
        }

        let parts: Vec<&str> = stele.split(sep).collect();
        let mut values: HashMap<String, StateValue> = HashMap::new();

        // Skip header, process rest
        for part in parts.iter().skip(1) {
            if part.is_empty() {
                continue;
            }

            // Check if this starts with a nested parent symbol followed by the nested separator
            let mut is_nested = false;
            let mut parent_sym_len = 0;

            for parent_sym in &nested_parent_syms {
                let pattern = format!("{}{}", parent_sym, nsep);
                if part.starts_with(&pattern) {
                    is_nested = true;
                    parent_sym_len = parent_sym.len();
                    break;
                }
            }

            if is_nested {
                // Nested dimension: {parent_sym}{nsep}{child_sym}{value}
                let parent_part = &part[..parent_sym_len];
                let child_part = &part[parent_sym_len + nsep.len()..];

                if let Some(&parent_name) = top_level_sym_to_name.get(parent_part)
                    && let Some(Dimension::Nested { dimensions, .. }) =
                        schema.dimensions.get(parent_name)
                {
                    // Build child symbol map
                    let mut child_sym_to_name: HashMap<&str, &str> = HashMap::new();
                    for child_name in dimensions.keys() {
                        if let Some(child_sym) = s.symbols.get(child_name.as_str()) {
                            child_sym_to_name.insert(child_sym.as_str(), child_name.as_str());
                        }
                    }

                    // Find which child this is
                    for (child_sym, &child_name) in &child_sym_to_name {
                        if let Some(value_str) = child_part.strip_prefix(child_sym) {
                            // Get or create nested HashMap
                            let nested = values
                                .entry(parent_name.to_string())
                                .or_insert_with(|| StateValue::Nested(HashMap::new()));

                            if let StateValue::Nested(nested_map) = nested
                                && let Some(child_dim) = dimensions.get(child_name)
                            {
                                match child_dim {
                                    Dimension::Float { .. } => {
                                        if let Ok(v) = value_str.parse::<f32>() {
                                            nested_map.insert(
                                                child_name.to_string(),
                                                StateValue::Float(v),
                                            );
                                        }
                                    }
                                    Dimension::Enum { .. } => {
                                        let enum_val = rev_modality
                                            .get(value_str)
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| value_str.to_string());
                                        nested_map.insert(
                                            child_name.to_string(),
                                            StateValue::Enum(enum_val),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                            break;
                        }
                    }
                }
            } else {
                // Simple dimension
                for (sym, &name) in &top_level_sym_to_name {
                    if let Some(value_str) = part.strip_prefix(sym) {
                        if let Some(dim) = schema.dimensions.get(name) {
                            match dim {
                                Dimension::Float { .. } => {
                                    if let Ok(v) = value_str.parse::<f32>() {
                                        values.insert(name.to_string(), StateValue::Float(v));
                                    }
                                }
                                Dimension::Enum { .. } => {
                                    let enum_val = rev_modality
                                        .get(value_str)
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| value_str.to_string());
                                    values.insert(name.to_string(), StateValue::Enum(enum_val));
                                }
                                Dimension::Nested { .. } => {
                                    // Skip nested dimensions in simple branch
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        Ok(DynamicState {
            schema_id: schema.title.clone(),
            values,
        })
    }
    /// Create DynamicState from a discrete mode name using schema mappings
    pub fn from_mode(mode: &str, schema: &StateSchema) -> Result<Self> {
        let mapping = schema
            .mode_mappings
            .get(mode)
            .or_else(|| schema.mode_mappings.get("default"))
            .context("No mode mapping found and no default defined")?;

        // Clone the mapping values directly - they're already StateValue
        let values = mapping.clone();

        Ok(DynamicState {
            schema_id: schema.title.clone(),
            values,
        })
    }

    /// Generate human-readable description from DynamicState
    pub fn describe(&self, schema: &StateSchema) -> String {
        let mut parts = Vec::new();

        // Get dimension names in sorted order
        let mut dim_names: Vec<_> = self.values.keys().collect();
        dim_names.sort();

        for dim_name in dim_names {
            if let Some(value) = self.values.get(dim_name.as_str())
                && let Some(dim_def) = schema.dimensions.get(dim_name.as_str())
            {
                let desc = Self::describe_value(dim_name, dim_def, value);
                if !desc.is_empty() {
                    parts.push(desc);
                }
            }
        }

        parts.join(", ")
    }

    fn describe_value(name: &str, definition: &Dimension, value: &StateValue) -> String {
        match (definition, value) {
            (Dimension::Float { hints, .. }, StateValue::Float(v)) => {
                // Find closest hint
                let hint_name = if hints.is_empty() {
                    String::new()
                } else {
                    let mut closest = ("", f32::MAX);
                    for (hint_name, hint_val) in hints {
                        let distance = (v - hint_val).abs();
                        if distance < closest.1 {
                            closest = (hint_name.as_str(), distance);
                        }
                    }
                    closest.0.to_string()
                };

                if hint_name.is_empty() {
                    format!("{}: {:.1}", name, v)
                } else {
                    format!("{}: {} ({:.1})", name, hint_name, v)
                }
            }
            (Dimension::Enum { .. }, StateValue::Enum(v)) => {
                format!("{}: {}", name, v)
            }
            (Dimension::Nested { dimensions, .. }, StateValue::Nested(nested_values)) => {
                let mut nested_parts = Vec::new();

                // Get nested dimension names in sorted order
                let mut nested_names: Vec<_> = nested_values.keys().collect();
                nested_names.sort();

                for nested_name in nested_names {
                    if let Some(nested_value) = nested_values.get(nested_name.as_str())
                        && let Some(nested_def) = dimensions.get(nested_name.as_str())
                    {
                        let desc = Self::describe_value(nested_name, nested_def, nested_value);
                        if !desc.is_empty() {
                            nested_parts.push(desc);
                        }
                    }
                }

                if nested_parts.is_empty() {
                    String::new()
                } else {
                    format!("{}: [{}]", name, nested_parts.join(", "))
                }
            }
            _ => String::new(),
        }
    }

    /// Interactive state capture - prompts for each dimension based on schema
    pub fn interactive_capture(schema: &StateSchema) -> Result<Self> {
        use std::io::{self, Write};

        fn prompt_float(prompt: &str, hints: &HashMap<String, f32>) -> Result<f32> {
            let hint_str: Vec<String> = hints
                .iter()
                .map(|(k, v)| format!("{}={:.1}", k, v))
                .collect();

            print!("{} [{}]: ", prompt, hint_str.join(", "));
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();

            // Check if it matches a hint
            if let Some(&val) = hints.get(input) {
                return Ok(val);
            }

            // Try to parse as float
            input.parse().context("Expected number or hint word")
        }

        fn prompt_enum(prompt: &str, values: &[String]) -> Result<String> {
            print!("{} [{}]: ", prompt, values.join("/"));
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim().to_lowercase();

            if values.iter().any(|v| v.to_lowercase() == input) {
                Ok(input)
            } else {
                bail!("Expected one of: {}", values.join(", "))
            }
        }

        fn capture_dimension(_name: &str, definition: &Dimension) -> Result<StateValue> {
            match definition {
                Dimension::Float { prompt, hints, .. } => {
                    let value = prompt_float(prompt, hints)?;
                    Ok(StateValue::Float(value))
                }
                Dimension::Enum { prompt, values, .. } => {
                    let value = prompt_enum(prompt, values)?;
                    Ok(StateValue::Enum(value))
                }
                Dimension::Nested {
                    description,
                    dimensions,
                } => {
                    println!("\n{}", description);
                    let mut nested_values = HashMap::new();

                    // Get nested dimension names in sorted order
                    let mut nested_names: Vec<_> = dimensions.keys().collect();
                    nested_names.sort();

                    for nested_name in nested_names {
                        if let Some(nested_def) = dimensions.get(nested_name.as_str()) {
                            let nested_value = capture_dimension(nested_name, nested_def)?;
                            nested_values.insert(nested_name.to_string(), nested_value);
                        }
                    }

                    Ok(StateValue::Nested(nested_values))
                }
            }
        }

        println!("\n{}: {}\n", schema.title, schema.description);

        let mut values = HashMap::new();

        // Get dimension names in sorted order
        let mut dim_names: Vec<_> = schema.dimensions.keys().collect();
        dim_names.sort();

        for dim_name in dim_names {
            if let Some(dim_def) = schema.dimensions.get(dim_name.as_str()) {
                let value = capture_dimension(dim_name, dim_def)?;
                values.insert(dim_name.to_string(), value);
            }
        }

        Ok(DynamicState {
            schema_id: schema.title.clone(),
            values,
        })
    }
}

/// Stele encoding configuration from schema
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteleConfig {
    pub header: String,
    pub separator: String,
    pub nested_separator: String,
    pub symbols: HashMap<String, String>,
    pub modality_values: HashMap<String, String>,
}

/// Dimension hint - maps word to float value
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DimensionHints {
    #[serde(flatten)]
    pub values: HashMap<String, f32>,
}

/// A single dimension definition
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Dimension {
    #[serde(rename = "float")]
    Float {
        range: [f32; 2],
        description: String,
        prompt: String,
        #[serde(default)]
        hints: HashMap<String, f32>,
    },
    #[serde(rename = "enum")]
    Enum {
        values: Vec<String>,
        description: String,
        prompt: String,
    },
    #[serde(rename = "nested")]
    Nested {
        description: String,
        dimensions: HashMap<String, Dimension>,
    },
}

/// Mode mapping - predefined tensor values for discrete modes (schema-agnostic)
pub type ModeMapping = HashMap<String, StateValue>;

/// The full schema definition
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StateSchema {
    pub title: String,
    pub description: String,
    pub version: String,
    #[serde(rename = "type")]
    pub schema_type: String,
    pub name: String,
    pub stele: SteleConfig,
    pub dimensions: HashMap<String, Dimension>,
    pub mode_mappings: HashMap<String, ModeMapping>,
}
