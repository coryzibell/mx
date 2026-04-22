//! Legacy concrete state types
//!
//! Contains `EmotionalState` and `TowardState` -- the original hard-coded state
//! representations that predate the schema-driven `DynamicState` system.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::schema::{Dimension, DynamicState, StateSchema, StateValue};

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

        // Format: @state:crewu|T0.7|E0.3|G0.6|D0.5|N0.8|>.A0.4|>.F0.6|>.I0.2|>.M{modality_symbol}
        format!(
            "{}:{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            s.header,
            schema.name,
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

/// Parse a wake preference line and convert to EmotionalState
/// Handles both old format (Wake Preference: soft) and new stele format
/// DEPRECATED: Use parse_wake_preference_dynamic for schema-agnostic parsing
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
