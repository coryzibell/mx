//! Schema-agnostic State Tensor Encoding System
//!
//! Implements the tensor encoding system designed by Schemnya.
//! Values-first: encode dimensional values -> derive nearest mood label.
//!
//! The crewu schema defines 5 dimensions:
//! - temperature (ᚣ): cold/precise <-> warm/playful
//! - entropy (ᚤ): ordered/focused <-> chaotic/wild
//! - agency (ᚡ): receptive/yielding <-> active/driving
//! - connection (ᚢ): distant/separate <-> close/merged
//! - weight (ᚠ): light/floating <-> heavy/grounded

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A dimension definition in a tensor schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorDimension {
    /// Unique identifier (e.g., "temperature")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Optional decorative rune
    #[serde(default)]
    pub rune: Option<String>,
    /// Anchors for low/mid/high values
    pub anchors: DimensionAnchors,
    /// Default value (0.0-1.0)
    #[serde(default = "default_half")]
    pub default: f32,
}

fn default_half() -> f32 {
    0.5
}

/// Anchor descriptions for a dimension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionAnchors {
    /// Description for low values (near 0.0)
    pub low: String,
    /// Description for mid values (near 0.5)
    #[serde(default)]
    pub mid: Option<String>,
    /// Description for high values (near 1.0)
    pub high: String,
}

/// A mood definition - a named landmark in the state space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorMood {
    /// Human-readable description
    pub description: String,
    /// Canonical tensor values (positional, matching dimension order)
    pub tensor: Vec<f32>,
    /// Which dimensions matter most for this mood (weights)
    #[serde(default)]
    pub weights: Option<Vec<f32>>,
    /// How far from canonical tensor still counts as this mood
    #[serde(default = "default_tolerance")]
    pub tolerance: f32,
}

fn default_tolerance() -> f32 {
    0.3
}

/// The full tensor schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TensorSchema {
    /// Schema identifier (e.g., "crewu")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Schema version
    #[serde(default = "default_version")]
    pub version: u32,
    /// Ordered list of dimensions
    pub dimensions: Vec<TensorDimension>,
    /// Named moods (landmarks in the space)
    #[serde(default)]
    pub moods: HashMap<String, TensorMood>,
}

fn default_version() -> u32 {
    1
}

impl TensorSchema {
    /// Load schema from a YAML file
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read schema file: {:?}", path))?;

        // Try YAML first, fall back to JSON
        serde_yaml::from_str(&content)
            .or_else(|_| serde_json::from_str(&content))
            .with_context(|| format!("Failed to parse schema: {:?}", path))
    }

    /// Load schema by ID from standard locations
    pub fn load_by_id(schema_id: &str) -> Result<Self> {
        // Check env var first
        if let Ok(schema_path) = std::env::var("MX_STATE_SCHEMA") {
            let path = std::path::PathBuf::from(&schema_path);
            if path.exists() {
                let schema = Self::load(&path)?;
                if schema.id == schema_id {
                    return Ok(schema);
                }
            }
        }

        // Check ~/.crewu/schemas/{id}.yaml
        if let Some(home) = dirs::home_dir() {
            let yaml_path = home.join(format!(".crewu/schemas/{}.yaml", schema_id));
            if yaml_path.exists() {
                return Self::load(&yaml_path);
            }

            let json_path = home.join(format!(".crewu/schemas/{}.json", schema_id));
            if json_path.exists() {
                return Self::load(&json_path);
            }
        }

        bail!("Schema '{}' not found in ~/.crewu/schemas/", schema_id)
    }

    /// Load default schema (from MX_STATE_SCHEMA or crewu)
    pub fn load_default() -> Result<Self> {
        // Check MX_STATE_SCHEMA env var
        if let Ok(schema_path) = std::env::var("MX_STATE_SCHEMA") {
            let path = std::path::PathBuf::from(&schema_path);
            if path.exists() {
                return Self::load(&path);
            }
        }

        // Try crewu as default
        Self::load_by_id("crewu")
    }

    /// List available schemas in ~/.crewu/schemas/
    pub fn list_available() -> Result<Vec<String>> {
        let mut schemas = Vec::new();

        if let Some(home) = dirs::home_dir() {
            let schemas_dir = home.join(".crewu/schemas");
            if schemas_dir.exists() {
                for entry in fs::read_dir(&schemas_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if let Some(ext) = path.extension()
                        && (ext == "yaml" || ext == "yml" || ext == "json")
                        && let Some(stem) = path.file_stem()
                    {
                        schemas.push(stem.to_string_lossy().to_string());
                    }
                }
            }
        }

        schemas.sort();
        schemas.dedup();
        Ok(schemas)
    }

    /// Get dimension by index
    pub fn dimension(&self, index: usize) -> Option<&TensorDimension> {
        self.dimensions.get(index)
    }

    /// Get dimension by ID
    pub fn dimension_by_id(&self, id: &str) -> Option<(usize, &TensorDimension)> {
        self.dimensions.iter().enumerate().find(|(_, d)| d.id == id)
    }

    /// Number of dimensions
    pub fn dimension_count(&self) -> usize {
        self.dimensions.len()
    }
}

/// An encoded state tensor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTensor {
    /// Schema ID this tensor belongs to
    pub schema_id: String,
    /// Values for each dimension (positional)
    pub values: Vec<f32>,
}

impl StateTensor {
    /// Create a new state tensor with explicit values
    pub fn new(schema_id: String, values: Vec<f32>) -> Self {
        Self { schema_id, values }
    }

    /// Create a state tensor with default values from schema
    pub fn default_from_schema(schema: &TensorSchema) -> Self {
        let values: Vec<f32> = schema.dimensions.iter().map(|d| d.default).collect();
        Self {
            schema_id: schema.id.clone(),
            values,
        }
    }

    /// Parse values from a pipe-separated string
    /// Format: "0.3|0.2|0.7|0.8|0.4"
    pub fn parse_values(schema: &TensorSchema, input: &str) -> Result<Self> {
        let parts: Vec<&str> = input.split('|').collect();

        if parts.len() != schema.dimension_count() {
            bail!(
                "Expected {} values for schema '{}', got {}",
                schema.dimension_count(),
                schema.id,
                parts.len()
            );
        }

        let mut values = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            let dim = &schema.dimensions[i];
            let value: f32 = part
                .trim()
                .parse()
                .with_context(|| format!("Invalid value for dimension '{}': {}", dim.id, part))?;

            // Clamp to 0.0-1.0 range
            values.push(value.clamp(0.0, 1.0));
        }

        Ok(Self {
            schema_id: schema.id.clone(),
            values,
        })
    }

    /// Encode to the standard string format
    /// Format: @state:crewu|0.3|0.2|0.7|0.8|0.4
    pub fn encode(&self) -> String {
        let values_str: Vec<String> = self.values.iter().map(|v| format!("{:.2}", v)).collect();
        format!("@state:{}|{}", self.schema_id, values_str.join("|"))
    }

    /// Encode with optional rune decoration
    /// Format: @state:crewu|ᚣ0.30|ᚤ0.20|ᚡ0.70|ᚢ0.80|ᚠ0.40
    pub fn encode_with_runes(&self, schema: &TensorSchema) -> String {
        let parts: Vec<String> = self
            .values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let rune = schema
                    .dimensions
                    .get(i)
                    .and_then(|d| d.rune.as_ref())
                    .map(|r| r.as_str())
                    .unwrap_or("");
                format!("{}{:.2}", rune, v)
            })
            .collect();
        format!("@state:{}|{}", self.schema_id, parts.join("|"))
    }

    /// Decode from standard string format
    pub fn decode(input: &str) -> Result<Self> {
        let input = input.trim();

        if !input.starts_with("@state:") {
            bail!("Invalid tensor format: must start with @state:");
        }

        let rest = &input[7..]; // Skip "@state:"
        let parts: Vec<&str> = rest.split('|').collect();

        if parts.is_empty() {
            bail!("Invalid tensor format: missing schema ID");
        }

        let schema_id = parts[0].to_string();

        let mut values = Vec::with_capacity(parts.len() - 1);
        for part in parts.iter().skip(1) {
            // Strip any rune prefix (non-digit, non-dot characters)
            let value_str: String = part
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '.' && *c != '-')
                .collect();

            let value: f32 = value_str
                .parse()
                .with_context(|| format!("Invalid value: {}", part))?;
            values.push(value.clamp(0.0, 1.0));
        }

        Ok(Self { schema_id, values })
    }

    /// Calculate weighted Euclidean distance to a mood
    pub fn distance_to_mood(&self, mood: &TensorMood) -> f32 {
        if self.values.len() != mood.tensor.len() {
            return f32::MAX;
        }

        let weights = mood.weights.as_ref();

        let sum: f32 = self
            .values
            .iter()
            .zip(mood.tensor.iter())
            .enumerate()
            .map(|(i, (v1, v2))| {
                let weight = weights.and_then(|w| w.get(i)).copied().unwrap_or(1.0);
                weight * (v1 - v2).powi(2)
            })
            .sum();

        sum.sqrt()
    }

    /// Find the nearest mood within tolerance
    pub fn nearest_mood<'a>(
        &self,
        schema: &'a TensorSchema,
    ) -> Option<(&'a str, &'a TensorMood, f32)> {
        let mut nearest: Option<(&str, &TensorMood, f32)> = None;

        for (name, mood) in &schema.moods {
            let distance = self.distance_to_mood(mood);

            if distance <= mood.tolerance {
                match &nearest {
                    None => nearest = Some((name.as_str(), mood, distance)),
                    Some((_, _, prev_dist)) if distance < *prev_dist => {
                        nearest = Some((name.as_str(), mood, distance));
                    }
                    _ => {}
                }
            }
        }

        nearest
    }

    /// Generate a human-readable description
    pub fn describe(&self, schema: &TensorSchema) -> String {
        let mut parts = Vec::new();

        for (i, value) in self.values.iter().enumerate() {
            if let Some(dim) = schema.dimensions.get(i) {
                let anchor_desc = if *value < 0.33 {
                    &dim.anchors.low
                } else if *value > 0.66 {
                    &dim.anchors.high
                } else {
                    dim.anchors.mid.as_ref().unwrap_or(&dim.anchors.low)
                };
                parts.push(format!("{}: {:.2} ({})", dim.name, value, anchor_desc));
            }
        }

        parts.join(", ")
    }

    /// Get value for a dimension by ID
    pub fn get(&self, schema: &TensorSchema, dim_id: &str) -> Option<f32> {
        schema
            .dimension_by_id(dim_id)
            .and_then(|(idx, _)| self.values.get(idx))
            .copied()
    }

    /// Set value for a dimension by ID
    pub fn set(&mut self, schema: &TensorSchema, dim_id: &str, value: f32) -> Result<()> {
        let (idx, _) = schema
            .dimension_by_id(dim_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown dimension: {}", dim_id))?;

        if idx < self.values.len() {
            self.values[idx] = value.clamp(0.0, 1.0);
        }

        Ok(())
    }
}

/// Interactive guided state capture
pub fn guided_capture(schema: &TensorSchema) -> Result<StateTensor> {
    use std::io::{self, Write};

    println!("\n{} ({})\n", schema.name, schema.id);
    println!("Enter values 0.0-1.0 for each dimension.\n");

    let mut values = Vec::with_capacity(schema.dimensions.len());

    for dim in &schema.dimensions {
        let rune = dim
            .rune
            .as_ref()
            .map(|r| format!("{} ", r))
            .unwrap_or_default();

        println!("{}{}:", rune, dim.name);
        println!("  Low (0.0): {}", dim.anchors.low);
        if let Some(mid) = &dim.anchors.mid {
            println!("  Mid (0.5): {}", mid);
        }
        println!("  High (1.0): {}", dim.anchors.high);
        println!("  Default: {:.2}", dim.default);

        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        let value: f32 = if input.is_empty() {
            dim.default
        } else {
            input
                .parse()
                .with_context(|| format!("Invalid number: {}", input))?
        };

        values.push(value.clamp(0.0, 1.0));
        println!();
    }

    Ok(StateTensor {
        schema_id: schema.id.clone(),
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_schema() -> TensorSchema {
        TensorSchema {
            id: "test".to_string(),
            name: "Test Schema".to_string(),
            version: 1,
            dimensions: vec![
                TensorDimension {
                    id: "dim1".to_string(),
                    name: "Dimension 1".to_string(),
                    rune: Some("A".to_string()),
                    anchors: DimensionAnchors {
                        low: "low1".to_string(),
                        mid: Some("mid1".to_string()),
                        high: "high1".to_string(),
                    },
                    default: 0.5,
                },
                TensorDimension {
                    id: "dim2".to_string(),
                    name: "Dimension 2".to_string(),
                    rune: Some("B".to_string()),
                    anchors: DimensionAnchors {
                        low: "low2".to_string(),
                        mid: None,
                        high: "high2".to_string(),
                    },
                    default: 0.5,
                },
            ],
            moods: HashMap::from([
                (
                    "calm".to_string(),
                    TensorMood {
                        description: "Calm state".to_string(),
                        tensor: vec![0.2, 0.3],
                        weights: Some(vec![1.0, 0.8]),
                        tolerance: 0.3,
                    },
                ),
                (
                    "excited".to_string(),
                    TensorMood {
                        description: "Excited state".to_string(),
                        tensor: vec![0.8, 0.9],
                        weights: Some(vec![0.9, 1.0]),
                        tolerance: 0.3,
                    },
                ),
            ]),
        }
    }

    #[test]
    fn test_parse_values() {
        let schema = test_schema();
        let tensor = StateTensor::parse_values(&schema, "0.3|0.7").unwrap();

        assert_eq!(tensor.schema_id, "test");
        assert_eq!(tensor.values.len(), 2);
        assert!((tensor.values[0] - 0.3).abs() < 0.01);
        assert!((tensor.values[1] - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let tensor = StateTensor::new("crewu".to_string(), vec![0.3, 0.2, 0.7, 0.8, 0.4]);
        let encoded = tensor.encode();

        assert!(encoded.starts_with("@state:crewu|"));

        let decoded = StateTensor::decode(&encoded).unwrap();
        assert_eq!(decoded.schema_id, "crewu");
        assert_eq!(decoded.values.len(), 5);

        for (a, b) in tensor.values.iter().zip(decoded.values.iter()) {
            assert!((a - b).abs() < 0.01);
        }
    }

    #[test]
    fn test_nearest_mood() {
        let schema = test_schema();
        let tensor = StateTensor::new("test".to_string(), vec![0.25, 0.35]);

        let (name, _, distance) = tensor.nearest_mood(&schema).unwrap();
        assert_eq!(name, "calm");
        assert!(distance < 0.3);
    }

    #[test]
    fn test_distance_with_weights() {
        let schema = test_schema();

        // Close to calm but with different dimensions
        let tensor1 = StateTensor::new("test".to_string(), vec![0.2, 0.5]);
        let tensor2 = StateTensor::new("test".to_string(), vec![0.4, 0.3]);

        let calm = schema.moods.get("calm").unwrap();
        let dist1 = tensor1.distance_to_mood(calm);
        let dist2 = tensor2.distance_to_mood(calm);

        // tensor1 should be closer because dim2 has lower weight (0.8 vs 1.0)
        assert!(dist1 < dist2);
    }
}
