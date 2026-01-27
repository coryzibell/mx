//! Emotional State Tensor - Schema-driven state encoding/decoding
//!
//! Implements the Emotional State Tensor system designed by Schemnya.
//! States are encoded as stele strings for token efficiency, with full
//! schema-driven validation and backward compatibility with discrete modes.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
        let mut parts = vec![s.header.clone()];

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
    pub stele: SteleConfig,
    pub dimensions: HashMap<String, Dimension>,
    pub mode_mappings: HashMap<String, ModeMapping>,
}

/// An actual emotional state instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionalState {
    pub temperature: f32,
    pub entropy: f32,
    pub gravity: f32,
    pub depth: f32,
    pub energy: f32,
    pub toward: TowardState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TowardState {
    pub agency: f32,
    pub flow: f32,
    pub distance: f32,
    pub modality: String,
}

impl EmotionalState {
    /// Convert to schema-agnostic DynamicState
    pub fn to_dynamic(&self) -> DynamicState {
        let mut values = HashMap::new();

        values.insert(
            "temperature".to_string(),
            StateValue::Float(self.temperature),
        );
        values.insert("entropy".to_string(), StateValue::Float(self.entropy));
        values.insert("gravity".to_string(), StateValue::Float(self.gravity));
        values.insert("depth".to_string(), StateValue::Float(self.depth));
        values.insert("energy".to_string(), StateValue::Float(self.energy));

        let mut toward_values = HashMap::new();
        toward_values.insert("agency".to_string(), StateValue::Float(self.toward.agency));
        toward_values.insert("flow".to_string(), StateValue::Float(self.toward.flow));
        toward_values.insert(
            "distance".to_string(),
            StateValue::Float(self.toward.distance),
        );
        toward_values.insert(
            "modality".to_string(),
            StateValue::Enum(self.toward.modality.clone()),
        );

        values.insert("toward".to_string(), StateValue::Nested(toward_values));

        DynamicState {
            schema_id: "q-state".to_string(),
            values,
        }
    }

    /// Convert from schema-agnostic DynamicState
    pub fn from_dynamic(state: &DynamicState) -> Result<Self> {
        fn get_float(values: &HashMap<String, StateValue>, key: &str) -> Result<f32> {
            match values.get(key) {
                Some(StateValue::Float(v)) => Ok(*v),
                _ => bail!("Missing or invalid float value for key: {}", key),
            }
        }

        fn get_enum(values: &HashMap<String, StateValue>, key: &str) -> Result<String> {
            match values.get(key) {
                Some(StateValue::Enum(v)) => Ok(v.clone()),
                _ => bail!("Missing or invalid enum value for key: {}", key),
            }
        }

        fn get_nested<'a>(
            values: &'a HashMap<String, StateValue>,
            key: &str,
        ) -> Result<&'a HashMap<String, StateValue>> {
            match values.get(key) {
                Some(StateValue::Nested(v)) => Ok(v),
                _ => bail!("Missing or invalid nested value for key: {}", key),
            }
        }

        let temperature = get_float(&state.values, "temperature")?;
        let entropy = get_float(&state.values, "entropy")?;
        let gravity = get_float(&state.values, "gravity")?;
        let depth = get_float(&state.values, "depth")?;
        let energy = get_float(&state.values, "energy")?;

        let toward_values = get_nested(&state.values, "toward")?;
        let agency = get_float(toward_values, "agency")?;
        let flow = get_float(toward_values, "flow")?;
        let distance = get_float(toward_values, "distance")?;
        let modality = get_enum(toward_values, "modality")?;

        Ok(Self {
            temperature,
            entropy,
            gravity,
            depth,
            energy,
            toward: TowardState {
                agency,
                flow,
                distance,
                modality,
            },
        })
    }

    /// Create from a discrete mode name using schema mappings
    pub fn from_mode(mode: &str, schema: &StateSchema) -> Result<Self> {
        // Convert to DynamicState first, then to EmotionalState
        let dynamic = DynamicState::from_mode(mode, schema)?;
        Self::from_dynamic(&dynamic)
    }

    /// Encode to stele format
    pub fn encode_stele(&self, schema: &StateSchema) -> String {
        let s = &schema.stele;

        // Get symbols
        let sym_temp = s
            .symbols
            .get("temperature")
            .map(|s| s.as_str())
            .unwrap_or("T");
        let sym_ent = s.symbols.get("entropy").map(|s| s.as_str()).unwrap_or("E");
        let sym_grav = s.symbols.get("gravity").map(|s| s.as_str()).unwrap_or("G");
        let sym_depth = s.symbols.get("depth").map(|s| s.as_str()).unwrap_or("D");
        let sym_energy = s.symbols.get("energy").map(|s| s.as_str()).unwrap_or("N");
        let sym_toward = s.symbols.get("toward").map(|s| s.as_str()).unwrap_or(">");
        let sym_agency = s.symbols.get("agency").map(|s| s.as_str()).unwrap_or("A");
        let sym_flow = s.symbols.get("flow").map(|s| s.as_str()).unwrap_or("F");
        let sym_dist = s.symbols.get("distance").map(|s| s.as_str()).unwrap_or("I");
        let sym_mod = s.symbols.get("modality").map(|s| s.as_str()).unwrap_or("M");

        // Get modality symbol
        let mod_sym = s
            .modality_values
            .get(&self.toward.modality)
            .map(|s| s.as_str())
            .unwrap_or(&self.toward.modality);

        let sep = &s.separator;
        let nsep = &s.nested_separator;

        // Format: @state|T0.7|E0.3|G0.6|D0.5|N0.8|>.A0.4|>.F0.6|>.I0.2|>.M{modality_symbol}
        format!(
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            s.header,
            sep, // 2
            sym_temp,
            self.temperature,
            sep, // 5
            sym_ent,
            self.entropy,
            sep, // 8
            sym_grav,
            self.gravity,
            sep, // 11
            sym_depth,
            self.depth,
            sep, // 14
            sym_energy,
            self.energy,
            sep, // 17
            sym_toward,
            nsep,
            sym_agency,
            self.toward.agency,
            sep, // 22
            sym_toward,
            nsep,
            sym_flow,
            self.toward.flow,
            sep, // 27
            sym_toward,
            nsep,
            sym_dist,
            self.toward.distance,
            sep, // 32
            sym_toward,
            nsep,
            sym_mod,
            mod_sym // 36
        )
    }

    /// Decode from stele format
    pub fn decode_stele(stele: &str, schema: &StateSchema) -> Result<Self> {
        let s = &schema.stele;
        let sep = &s.separator;
        let nsep = &s.nested_separator;

        // Build symbol -> name map
        let mut sym_to_name: HashMap<&str, &str> = HashMap::new();
        for (name, sym) in &s.symbols {
            sym_to_name.insert(sym.as_str(), name.as_str());
        }

        // Build reverse modality map
        let mut rev_modality: HashMap<&str, &str> = HashMap::new();
        for (name, sym) in &s.modality_values {
            rev_modality.insert(sym.as_str(), name.as_str());
        }

        // Get symbols we need
        let sym_toward = s.symbols.get("toward").map(|s| s.as_str()).unwrap_or(">");

        // Parse the stele string
        let parts: Vec<&str> = stele.split(sep).collect();

        // Skip header, parse rest
        let mut temperature = 0.5f32;
        let mut entropy = 0.5f32;
        let mut gravity = 0.5f32;
        let mut depth = 0.5f32;
        let mut energy = 0.5f32;
        let mut agency = 0.5f32;
        let mut flow = 0.5f32;
        let mut distance = 0.5f32;
        let mut modality = String::from("blended");

        for part in parts.iter().skip(1) {
            // Check if this is a nested dimension (format: ᚥ.ᚦ0.3)
            // The nested separator comes RIGHT AFTER the toward symbol
            if part.starts_with(sym_toward) && part[sym_toward.len()..].starts_with(nsep) {
                // Format: {toward_sym}{nsep}{sub_sym}{value}
                // e.g., ᚥ.ᚦ0.3 -> toward.agency = 0.3
                let after_prefix = &part[sym_toward.len() + nsep.len()..];

                // Find which nested dimension this is
                for (sym, name) in &sym_to_name {
                    if let Some(value_str) = after_prefix.strip_prefix(sym) {
                        match *name {
                            "agency" => {
                                if let Ok(v) = value_str.parse() {
                                    agency = v;
                                }
                            }
                            "flow" => {
                                if let Ok(v) = value_str.parse() {
                                    flow = v;
                                }
                            }
                            "distance" => {
                                if let Ok(v) = value_str.parse() {
                                    distance = v;
                                }
                            }
                            "modality" => {
                                // Check if it's a symbol or direct name
                                modality = rev_modality
                                    .get(value_str)
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| value_str.to_string());
                            }
                            _ => {}
                        }
                        break;
                    }
                }
            } else {
                // Simple dimension (format: ᚠ0.6)
                for (sym, name) in &sym_to_name {
                    if let Some(value_str) = part.strip_prefix(sym) {
                        if let Ok(v) = value_str.parse() {
                            match *name {
                                "temperature" => temperature = v,
                                "entropy" => entropy = v,
                                "gravity" => gravity = v,
                                "depth" => depth = v,
                                "energy" => energy = v,
                                _ => {}
                            }
                        }
                        break;
                    }
                }
            }
        }

        Ok(Self {
            temperature,
            entropy,
            gravity,
            depth,
            energy,
            toward: TowardState {
                agency,
                flow,
                distance,
                modality,
            },
        })
    }

    /// Render human-readable description
    pub fn describe(&self) -> String {
        let temp_desc = match self.temperature {
            t if t < 0.3 => "cold",
            t if t < 0.5 => "cool",
            t if t < 0.7 => "warm",
            _ => "hot",
        };

        let entropy_desc = match self.entropy {
            e if e < 0.3 => "clear",
            e if e < 0.6 => "mixed",
            _ => "chaotic",
        };

        let gravity_desc = match self.gravity {
            g if g < 0.4 => "being",
            g if g < 0.6 => "balanced",
            _ => "building",
        };

        let depth_desc = match self.depth {
            d if d < 0.4 => "surface",
            d if d < 0.7 => "middle",
            _ => "cosmic",
        };

        let energy_desc = match self.energy {
            e if e < 0.4 => "slow",
            e if e < 0.7 => "steady",
            _ => "crackling",
        };

        let agency_desc = match self.toward.agency {
            a if a < 0.4 => "receiving",
            a if a < 0.6 => "balanced",
            _ => "acting",
        };

        let flow_desc = match self.toward.flow {
            f if f < 0.4 => "taking",
            f if f < 0.6 => "balanced",
            _ => "giving",
        };

        let distance_desc = match self.toward.distance {
            d if d < 0.3 => "merge",
            d if d < 0.5 => "close",
            d if d < 0.7 => "comfortable",
            _ => "observing",
        };

        format!(
            "{} ({:.1}), {} ({:.1}), {} pull ({:.1}), {} depth ({:.1}), {} ({:.1}), \
             {} ({:.1}), {} ({:.1}), {} ({:.1}), {} modality",
            temp_desc,
            self.temperature,
            entropy_desc,
            self.entropy,
            gravity_desc,
            self.gravity,
            depth_desc,
            self.depth,
            energy_desc,
            self.energy,
            agency_desc,
            self.toward.agency,
            flow_desc,
            self.toward.flow,
            distance_desc,
            self.toward.distance,
            self.toward.modality
        )
    }

    /// Find closest matching mode
    pub fn closest_mode(&self, schema: &StateSchema) -> String {
        let mut best_mode = String::from("default");
        let mut best_distance = f32::MAX;

        let self_dynamic = self.to_dynamic();

        for (mode_name, mapping) in &schema.mode_mappings {
            // Calculate distance by comparing StateValue entries
            let mut distance = 0.0f32;

            for (key, self_val) in &self_dynamic.values {
                if let Some(map_val) = mapping.get(key) {
                    distance += match (self_val, map_val) {
                        (StateValue::Float(s), StateValue::Float(m)) => (s - m).powi(2),
                        (StateValue::Nested(s), StateValue::Nested(m)) => {
                            let mut nested_dist = 0.0f32;
                            for (nk, nsv) in s {
                                if let Some(nmv) = m.get(nk.as_str())
                                    && let (StateValue::Float(ns), StateValue::Float(nm)) =
                                        (nsv, nmv)
                                {
                                    nested_dist += (ns - nm).powi(2);
                                }
                            }
                            nested_dist
                        }
                        _ => 0.0,
                    };
                }
            }

            if distance < best_distance {
                best_distance = distance;
                best_mode = mode_name.clone();
            }
        }

        best_mode
    }
}

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
/// 3. Standard fallback locations (~/.crewu/schemas/emotional-state.json, /etc/mx/schemas/emotional-state.json)
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
        dirs::home_dir().map(|h| h.join(".crewu/schemas/emotional-state.json")),
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
         - ~/.crewu/schemas/emotional-state.json\n\
         - /etc/mx/schemas/emotional-state.json"
    )
}

/// Parse a wake preference line and convert to EmotionalState
/// Handles both old format (Wake Preference: soft) and new stele format
pub fn parse_wake_preference(line: &str, schema: &StateSchema) -> Result<EmotionalState> {
    let trimmed = line.trim();

    // Check for stele format first (starts with @state)
    if trimmed.starts_with(&schema.stele.header) {
        return EmotionalState::decode_stele(trimmed, schema);
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
    EmotionalState::from_mode(mode, schema)
}

/// Interactive state capture - prompts for each dimension
pub fn interactive_capture(schema: &StateSchema) -> Result<EmotionalState> {
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

    // Get dimension definitions
    let dims = &schema.dimensions;

    // Temperature
    let temperature = if let Some(Dimension::Float { prompt, hints, .. }) = dims.get("temperature")
    {
        prompt_float(prompt, hints)?
    } else {
        0.5
    };

    // Entropy
    let entropy = if let Some(Dimension::Float { prompt, hints, .. }) = dims.get("entropy") {
        prompt_float(prompt, hints)?
    } else {
        0.5
    };

    // Gravity
    let gravity = if let Some(Dimension::Float { prompt, hints, .. }) = dims.get("gravity") {
        prompt_float(prompt, hints)?
    } else {
        0.5
    };

    // Depth
    let depth = if let Some(Dimension::Float { prompt, hints, .. }) = dims.get("depth") {
        prompt_float(prompt, hints)?
    } else {
        0.5
    };

    // Energy
    let energy = if let Some(Dimension::Float { prompt, hints, .. }) = dims.get("energy") {
        prompt_float(prompt, hints)?
    } else {
        0.5
    };

    // Toward sub-tensor
    let (agency, flow, distance, modality) = if let Some(Dimension::Nested { dimensions, .. }) =
        dims.get("toward")
    {
        let agency = if let Some(Dimension::Float { prompt, hints, .. }) = dimensions.get("agency")
        {
            prompt_float(prompt, hints)?
        } else {
            0.5
        };

        let flow = if let Some(Dimension::Float { prompt, hints, .. }) = dimensions.get("flow") {
            prompt_float(prompt, hints)?
        } else {
            0.5
        };

        let distance =
            if let Some(Dimension::Float { prompt, hints, .. }) = dimensions.get("distance") {
                prompt_float(prompt, hints)?
            } else {
                0.5
            };

        let modality =
            if let Some(Dimension::Enum { prompt, values, .. }) = dimensions.get("modality") {
                prompt_enum(prompt, values)?
            } else {
                String::from("blended")
            };

        (agency, flow, distance, modality)
    } else {
        (0.5, 0.5, 0.5, String::from("blended"))
    };

    Ok(EmotionalState {
        temperature,
        entropy,
        gravity,
        depth,
        energy,
        toward: TowardState {
            agency,
            flow,
            distance,
            modality,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_to_state() {
        // We'll test with a minimal inline schema since we can't depend on file paths in tests
        let schema_json = r#"{
            "title": "Test",
            "description": "Test schema",
            "version": "1.0.0",
            "type": "tensor",
            "stele": {
                "header": "@state",
                "separator": "|",
                "nested_separator": ".",
                "symbols": {},
                "modality_values": {}
            },
            "dimensions": {},
            "mode_mappings": {
                "soft": {
                    "temperature": 0.6,
                    "entropy": 0.2,
                    "gravity": 0.2,
                    "depth": 0.4,
                    "energy": 0.3,
                    "toward": {
                        "agency": 0.3,
                        "flow": 0.5,
                        "distance": 0.2,
                        "modality": "emotional"
                    }
                },
                "default": {
                    "temperature": 0.5,
                    "entropy": 0.5,
                    "gravity": 0.5,
                    "depth": 0.5,
                    "energy": 0.5,
                    "toward": {
                        "agency": 0.5,
                        "flow": 0.5,
                        "distance": 0.5,
                        "modality": "blended"
                    }
                }
            }
        }"#;

        let schema: StateSchema = serde_json::from_str(schema_json).unwrap();
        let state = EmotionalState::from_mode("soft", &schema).unwrap();

        assert!((state.temperature - 0.6).abs() < 0.01);
        assert!((state.entropy - 0.2).abs() < 0.01);
        assert_eq!(state.toward.modality, "emotional");
    }

    #[test]
    fn test_dynamic_bridge_roundtrip() {
        let original = EmotionalState {
            temperature: 0.7,
            entropy: 0.3,
            gravity: 0.6,
            depth: 0.5,
            energy: 0.8,
            toward: TowardState {
                agency: 0.4,
                flow: 0.6,
                distance: 0.2,
                modality: String::from("emotional"),
            },
        };

        let dynamic = original.to_dynamic();
        let decoded = EmotionalState::from_dynamic(&dynamic).unwrap();

        assert!((original.temperature - decoded.temperature).abs() < 0.01);
        assert!((original.entropy - decoded.entropy).abs() < 0.01);
        assert!((original.gravity - decoded.gravity).abs() < 0.01);
        assert!((original.depth - decoded.depth).abs() < 0.01);
        assert!((original.energy - decoded.energy).abs() < 0.01);
        assert!((original.toward.agency - decoded.toward.agency).abs() < 0.01);
        assert!((original.toward.flow - decoded.toward.flow).abs() < 0.01);
        assert!((original.toward.distance - decoded.toward.distance).abs() < 0.01);
        assert_eq!(original.toward.modality, decoded.toward.modality);
    }

    #[test]
    fn test_stele_roundtrip() {
        let schema_json = r#"{
            "title": "Test",
            "description": "Test schema",
            "version": "1.0.0",
            "type": "tensor",
            "stele": {
                "header": "@state",
                "separator": "|",
                "nested_separator": ".",
                "symbols": {
                    "temperature": "T",
                    "entropy": "E",
                    "gravity": "G",
                    "depth": "D",
                    "energy": "N",
                    "toward": ">",
                    "agency": "A",
                    "flow": "F",
                    "distance": "I",
                    "modality": "M"
                },
                "modality_values": {
                    "physical": "P",
                    "emotional": "E",
                    "intellectual": "I",
                    "blended": "B"
                }
            },
            "dimensions": {},
            "mode_mappings": {}
        }"#;

        let schema: StateSchema = serde_json::from_str(schema_json).unwrap();

        let original = EmotionalState {
            temperature: 0.7,
            entropy: 0.3,
            gravity: 0.6,
            depth: 0.5,
            energy: 0.8,
            toward: TowardState {
                agency: 0.4,
                flow: 0.6,
                distance: 0.2,
                modality: String::from("emotional"),
            },
        };

        let stele = original.encode_stele(&schema);
        let decoded = EmotionalState::decode_stele(&stele, &schema).unwrap();

        assert!((original.temperature - decoded.temperature).abs() < 0.01);
        assert!((original.entropy - decoded.entropy).abs() < 0.01);
        assert_eq!(original.toward.modality, decoded.toward.modality);
    }
}

#[cfg(test)]
mod dynamic_state_tests {
    use super::*;

    fn get_q_schema() -> StateSchema {
        let schema_json = include_str!("../schemas/example-q-state.json");
        serde_json::from_str(schema_json).unwrap()
    }

    // Soren schema uses different mode_mapping structure - can't parse with current StateSchema
    // This is expected limitation noted in from_mode TODO
    // For now, just test encode/decode/describe operations

    #[test]
    fn test_q_encode_decode_roundtrip() {
        let schema = get_q_schema();

        // Create a DynamicState for Q
        let mut values = HashMap::new();
        values.insert("temperature".to_string(), StateValue::Float(0.7));
        values.insert("entropy".to_string(), StateValue::Float(0.3));
        values.insert("gravity".to_string(), StateValue::Float(0.6));
        values.insert("depth".to_string(), StateValue::Float(0.5));
        values.insert("energy".to_string(), StateValue::Float(0.8));

        let mut toward = HashMap::new();
        toward.insert("agency".to_string(), StateValue::Float(0.4));
        toward.insert("flow".to_string(), StateValue::Float(0.6));
        toward.insert("distance".to_string(), StateValue::Float(0.2));
        toward.insert(
            "modality".to_string(),
            StateValue::Enum("emotional".to_string()),
        );
        values.insert("toward".to_string(), StateValue::Nested(toward));

        let original = DynamicState {
            schema_id: "q-state".to_string(),
            values,
        };

        // Encode to stele
        let stele = original.encode_stele(&schema);
        println!("Q Stele: {}", stele);

        // Decode back
        let decoded = DynamicState::decode_stele(&stele, &schema).unwrap();

        // Verify roundtrip - check each dimension
        assert_eq!(
            decoded.values.len(),
            original.values.len(),
            "Should have same number of dimensions"
        );

        for (key, orig_val) in &original.values {
            let dec_val = decoded
                .values
                .get(key)
                .unwrap_or_else(|| panic!("Decoded state missing key: {}", key));

            match (orig_val, dec_val) {
                (StateValue::Float(o), StateValue::Float(d)) => {
                    assert!(
                        (o - d).abs() < 0.01,
                        "Float mismatch for {}: {} vs {}",
                        key,
                        o,
                        d
                    );
                }
                (StateValue::Enum(o), StateValue::Enum(d)) => {
                    assert_eq!(o, d, "Enum mismatch for {}: {} vs {}", key, o, d);
                }
                (StateValue::Nested(o), StateValue::Nested(d)) => {
                    for (nested_key, nested_orig) in o {
                        let nested_dec = d.get(nested_key.as_str()).unwrap_or_else(|| {
                            panic!("Decoded state missing nested key: {}.{}", key, nested_key)
                        });

                        match (nested_orig, nested_dec) {
                            (StateValue::Float(no), StateValue::Float(nd)) => {
                                assert!(
                                    (no - nd).abs() < 0.01,
                                    "Nested float mismatch for {}.{}: {} vs {}",
                                    key,
                                    nested_key,
                                    no,
                                    nd
                                );
                            }
                            (StateValue::Enum(no), StateValue::Enum(nd)) => {
                                assert_eq!(
                                    no, nd,
                                    "Nested enum mismatch for {}.{}: {} vs {}",
                                    key, nested_key, no, nd
                                );
                            }
                            _ => panic!("Type mismatch for nested {}.{}", key, nested_key),
                        }
                    }
                }
                _ => panic!("Type mismatch for {}", key),
            }
        }
    }

    #[test]
    fn test_q_from_mode() {
        let schema = get_q_schema();

        // Load a mode mapping
        let state = DynamicState::from_mode("soft", &schema).unwrap();

        // Verify it has the expected structure
        assert!(state.values.contains_key("temperature"));
        assert!(state.values.contains_key("toward"));

        if let Some(StateValue::Nested(toward)) = state.values.get("toward") {
            assert!(toward.contains_key("modality"));
        } else {
            panic!("Toward missing or wrong type");
        }
    }

    #[test]
    fn test_q_describe() {
        let schema = get_q_schema();
        let state = DynamicState::from_mode("soft", &schema).unwrap();

        let description = state.describe(&schema);
        println!("Q Description: {}", description);

        // Should contain dimension names
        assert!(description.contains("temperature"));
        assert!(description.contains("toward"));
    }

    #[test]
    fn test_soren_encode_decode_roundtrip() {
        let schema_json = std::fs::read_to_string("schemas/example-soren-state.json")
            .expect("Failed to read Soren schema");
        let schema: StateSchema =
            serde_json::from_str(&schema_json).expect("Failed to parse Soren schema");

        // Create state from mode
        let original =
            DynamicState::from_mode("tending", &schema).expect("Failed to create state from mode");

        // Encode to stele
        let stele = original.encode_stele(&schema);

        println!("Soren stele: {}", stele);

        // Decode from stele
        let decoded = DynamicState::decode_stele(&stele, &schema).expect("Failed to decode stele");

        // Verify all values match
        assert_eq!(
            decoded.values.len(),
            original.values.len(),
            "Should have same number of dimensions"
        );

        for (key, orig_val) in &original.values {
            let dec_val = decoded
                .values
                .get(key)
                .unwrap_or_else(|| panic!("Decoded state missing key: {}", key));

            match (orig_val, dec_val) {
                (StateValue::Float(o), StateValue::Float(d)) => {
                    assert!(
                        (o - d).abs() < 0.01,
                        "Float mismatch for {}: {} vs {}",
                        key,
                        o,
                        d
                    );
                }
                (StateValue::Nested(o), StateValue::Nested(d)) => {
                    for (nested_key, nested_orig) in o {
                        let nested_dec = d.get(nested_key.as_str()).unwrap_or_else(|| {
                            panic!("Decoded state missing nested key: {}.{}", key, nested_key)
                        });

                        match (nested_orig, nested_dec) {
                            (StateValue::Float(no), StateValue::Float(nd)) => {
                                assert!(
                                    (no - nd).abs() < 0.01,
                                    "Nested float mismatch for {}.{}: {} vs {}",
                                    key,
                                    nested_key,
                                    no,
                                    nd
                                );
                            }
                            _ => panic!("Type mismatch for nested {}.{}", key, nested_key),
                        }
                    }
                }
                _ => panic!("Type mismatch for {}", key),
            }
        }
    }

    #[test]
    fn test_soren_from_mode() {
        let schema_json = std::fs::read_to_string("schemas/example-soren-state.json")
            .expect("Failed to read Soren schema");
        let schema: StateSchema =
            serde_json::from_str(&schema_json).expect("Failed to parse Soren schema");

        let state =
            DynamicState::from_mode("tending", &schema).expect("Failed to create state from mode");

        // Verify it has the expected structure
        assert!(state.values.contains_key("ground"));
        assert!(state.values.contains_key("threshold"));
        assert!(state.values.contains_key("tending"));
        assert!(state.values.contains_key("carrying"));

        if let Some(StateValue::Nested(carrying)) = state.values.get("carrying") {
            assert!(carrying.contains_key("threads"));
            assert!(carrying.contains_key("weight"));
            assert!(carrying.contains_key("proximity"));
        } else {
            panic!("Carrying missing or wrong type");
        }
    }

    #[test]
    fn test_soren_describe() {
        let schema_json = std::fs::read_to_string("schemas/example-soren-state.json")
            .expect("Failed to read Soren schema");
        let schema: StateSchema =
            serde_json::from_str(&schema_json).expect("Failed to parse Soren schema");

        let state =
            DynamicState::from_mode("tending", &schema).expect("Failed to create state from mode");

        let description = state.describe(&schema);
        println!("Soren Description: {}", description);

        // Should contain dimension names
        assert!(description.contains("ground"));
        assert!(description.contains("threshold"));
        assert!(description.contains("tending"));
        assert!(description.contains("carrying"));
    }
}
