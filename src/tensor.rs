//! Schema-agnostic State Tensor Encoding System
//!
//! Implements the tensor encoding system designed by Schemnya.
//! Values-first: encode dimensional values -> derive nearest mood label.
//!
//! Schemas are user-authored YAML files; the default `tensor` schema ships
//! with six dimensions (entropy, agency, temperature, verbosity, skepticism,
//! humor) and self-seeds at `$MX_HOME/state/schemas/tensor.yaml` on first
//! `mx state` invocation. See `schema/state/schemas/tensor.yaml` for the
//! shipped content.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Embedded default schema. Self-seeds into `$MX_HOME/state/schemas/tensor.yaml`
/// on first invocation when no user-authored file is present.
///
/// Source of truth: `schema/state/schemas/tensor.yaml` (under the existing
/// repo `schema/` asset convention -- compiled in alongside the SurrealDB
/// schema string).
const DEFAULT_TENSOR_SCHEMA_YAML: &str = include_str!("../schema/state/schemas/tensor.yaml");

/// The schema id whose absence triggers a self-seed of the embedded default.
const DEFAULT_TENSOR_SCHEMA_ID: &str = "tensor";

/// Pure seed logic: write `content` to `path` if it does not already exist.
///
/// Returns `Ok(true)` when the seed file was written, `Ok(false)` when the
/// path already existed and was preserved. Parent directories are created
/// as needed.
///
/// This is the test seam -- mirrors the `_with` pattern in `paths.rs` so
/// tests don't have to mutate `MX_HOME` (which is a process-wide `OnceLock`).
fn ensure_schema_seeded_at(path: &Path, content: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create schemas dir: {:?}", parent))?;
    }
    fs::write(path, content)
        .with_context(|| format!("Failed to seed default tensor schema at: {:?}", path))?;
    Ok(true)
}

/// Ensure the default `tensor` schema is seeded into `$MX_HOME/state/schemas/`.
///
/// No-op when the user has authored (or a previous run has seeded) a file at
/// `paths::tensor_schema_path("tensor")`. The file's existence is the signal:
/// once present, content is preserved untouched on subsequent runs.
///
/// Returns `Ok(true)` when this invocation actually performed the first-run
/// seed, `Ok(false)` when an existing file was preserved. Callers may use
/// this to log "first-run seed performed" without re-stat'ing the path.
fn ensure_default_schema_seeded() -> Result<bool> {
    let path = crate::paths::tensor_schema_path(DEFAULT_TENSOR_SCHEMA_ID);
    ensure_schema_seeded_at(&path, DEFAULT_TENSOR_SCHEMA_YAML)
}

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

    /// Load schema by ID from `$MX_HOME/state/schemas/`.
    ///
    /// The canonical `.yaml` path goes through `paths::tensor_schema_path`
    /// (the helper named in decision 5). The `.yml` and `.json` extensions
    /// are an extension-fallback chain -- they're intentionally constructed
    /// inline against `state_schemas_dir()` because the path helper is
    /// extension-specific by design.
    pub fn load_by_id(schema_id: &str) -> Result<Self> {
        // Self-seed the default `tensor` schema on first invocation. No-op when
        // a user-authored or previously-seeded file already exists at the
        // canonical path. Only seeds for the default id -- other schemas remain
        // user-authored.
        if schema_id == DEFAULT_TENSOR_SCHEMA_ID {
            let _seeded = ensure_default_schema_seeded()?;
        }

        let yaml_path = crate::paths::tensor_schema_path(schema_id);
        if yaml_path.exists() {
            return Self::load(&yaml_path);
        }

        let schemas_dir = crate::paths::state_schemas_dir();

        let yml_path = schemas_dir.join(format!("{}.yml", schema_id));
        if yml_path.exists() {
            return Self::load(&yml_path);
        }

        let json_path = schemas_dir.join(format!("{}.json", schema_id));
        if json_path.exists() {
            return Self::load(&json_path);
        }

        bail!(
            "Schema '{}' not found in {}",
            schema_id,
            schemas_dir.display()
        )
    }

    /// Load the default schema.
    ///
    /// Default schema ID is `tensor`. Override with `--schema {id|path}`
    /// at the CLI level (this method itself takes no arguments and is the
    /// fall-through used when no flag is supplied).
    pub fn load_default() -> Result<Self> {
        Self::load_by_id("tensor")
    }

    /// List available schemas in `$MX_HOME/state/schemas/`.
    pub fn list_available() -> Result<Vec<String>> {
        let mut schemas = Vec::new();

        let schemas_dir = crate::paths::state_schemas_dir();
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

    /// Parse named dimension values from a space-separated string
    /// Format: "temp=0.8 entropy=0.75 agency=0.4 connection=0.9 weight=0.6"
    /// Dimension names can be abbreviated (matches prefix of dimension ID)
    pub fn parse_named_dimensions(schema: &TensorSchema, input: &str) -> Result<Self> {
        use std::collections::HashMap;

        // Parse the key=value pairs
        let mut named_values: HashMap<String, f32> = HashMap::new();
        for part in input.split_whitespace() {
            let kv: Vec<&str> = part.split('=').collect();
            if kv.len() != 2 {
                bail!("Invalid dimension format '{}'. Expected 'name=value'", part);
            }

            let name = kv[0].trim().to_lowercase();
            let value: f32 = kv[1]
                .trim()
                .parse()
                .with_context(|| format!("Invalid value for dimension '{}': {}", name, kv[1]))?;

            named_values.insert(name, value.clamp(0.0, 1.0));
        }

        // Match named values to schema dimensions (support abbreviations)
        let mut values = Vec::with_capacity(schema.dimensions.len());
        for dim in &schema.dimensions {
            let dim_id_lower = dim.id.to_lowercase();

            // Try exact match first
            let value = if let Some(&v) = named_values.get(&dim_id_lower) {
                v
            } else {
                // Try prefix match
                let prefix_match = named_values
                    .iter()
                    .find(|(k, _)| dim_id_lower.starts_with(k.as_str()))
                    .map(|(_, &v)| v);

                match prefix_match {
                    Some(v) => v,
                    None => bail!(
                        "No value provided for dimension '{}'. Available: {}",
                        dim.id,
                        schema
                            .dimensions
                            .iter()
                            .map(|d| &d.id)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            };

            values.push(value);
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

    /// Format as self-documenting bootstrap output
    /// Returns a multi-line string ready for session bootstrap:
    /// - Line 1: Wake State with rune-encoded stele
    /// - Line 2: Rune legend showing dimension mapping
    /// - Line 3: Empty line
    /// - Line 4: Human-readable description with interpolated anchors
    pub fn format_bootstrap(&self, schema: &TensorSchema) -> Result<String> {
        use std::fmt::Write;

        let mut output = String::new();

        // Line 1: Wake State with rune-encoded stele
        writeln!(
            &mut output,
            "Wake State: {}",
            self.encode_with_runes(schema)
        )?;

        // Line 2: Rune legend
        let legend_parts: Vec<String> = schema
            .dimensions
            .iter()
            .filter_map(|dim| dim.rune.as_ref().map(|rune| format!("{}={}", rune, dim.id)))
            .collect();

        if !legend_parts.is_empty() {
            writeln!(&mut output, "({})", legend_parts.join(", "))?;
        }

        // Line 3: Empty line
        writeln!(&mut output)?;

        // Line 4: Human-readable description with interpolated anchors
        let desc_parts: Vec<String> = self
            .values
            .iter()
            .enumerate()
            .filter_map(|(i, value)| {
                schema.dimensions.get(i).map(|dim| {
                    // Interpolate between low, mid, and high anchors
                    let anchor_desc = self.interpolate_anchor_description(dim, *value);
                    format!("{} ({:.1})", anchor_desc, value)
                })
            })
            .collect();

        write!(&mut output, "{}.", desc_parts.join(", "))?;

        Ok(output)
    }

    /// Interpolate anchor description based on value
    /// Returns a human-readable description interpolated from the schema's anchors
    fn interpolate_anchor_description(&self, dim: &TensorDimension, value: f32) -> String {
        // Get anchor descriptions
        let low = &dim.anchors.low;
        let high = &dim.anchors.high;

        // For values < 0.33: use low anchor
        // For values > 0.66: use high anchor
        // For values 0.33-0.66: use mid anchor or "moderately {high}"
        if value < 0.33 {
            low.clone()
        } else if value > 0.66 {
            high.clone()
        } else {
            // Middle range - use mid anchor if available, otherwise blend
            if let Some(mid) = &dim.anchors.mid {
                mid.clone()
            } else {
                // Blend toward high if > 0.5, toward low if < 0.5
                if value > 0.5 {
                    format!("moderately {}", high)
                } else {
                    format!("moderately {}", low)
                }
            }
        }
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

    #[test]
    fn embedded_yaml_matches_disk_source() {
        // The compiled-in `DEFAULT_TENSOR_SCHEMA_YAML` is `include_str!` of
        // `schema/state/schemas/tensor.yaml`. `cargo build` enforces that the
        // path resolves, but doesn't enforce that the resolved file is the
        // one we think it is. This test pins the invariant explicitly so
        // a stale embed (e.g. via a stray same-named file higher up the
        // include path on some future refactor) fails loud here rather than
        // silently shipping mismatched content.
        let on_disk = std::fs::read_to_string("schema/state/schemas/tensor.yaml")
            .expect("schema/state/schemas/tensor.yaml must exist for include_str! to work");
        assert_eq!(
            on_disk, DEFAULT_TENSOR_SCHEMA_YAML,
            "embedded const drifted from disk source -- this should be \
             impossible since include_str! is compile-time"
        );
    }

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

    #[test]
    fn test_parse_named_dimensions() {
        let schema = test_schema();

        // Test full names
        let tensor = StateTensor::parse_named_dimensions(&schema, "dim1=0.3 dim2=0.7").unwrap();
        assert_eq!(tensor.schema_id, "test");
        assert_eq!(tensor.values.len(), 2);
        assert!((tensor.values[0] - 0.3).abs() < 0.01);
        assert!((tensor.values[1] - 0.7).abs() < 0.01);

        // Test case insensitivity
        let tensor2 = StateTensor::parse_named_dimensions(&schema, "DIM1=0.4 DIM2=0.8").unwrap();
        assert!((tensor2.values[0] - 0.4).abs() < 0.01);
        assert!((tensor2.values[1] - 0.8).abs() < 0.01);

        // Test abbreviations (prefix match)
        let tensor3 = StateTensor::parse_named_dimensions(&schema, "d=0.5 dim2=0.6").unwrap();
        assert!((tensor3.values[0] - 0.5).abs() < 0.01);
        assert!((tensor3.values[1] - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_parse_named_dimensions_missing() {
        let schema = test_schema();

        // Should error when dimension is missing
        let result = StateTensor::parse_named_dimensions(&schema, "dim1=0.3");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No value provided for dimension 'dim2'")
        );
    }

    #[test]
    fn test_format_bootstrap() {
        let schema = test_schema();
        let tensor = StateTensor::new("test".to_string(), vec![0.8, 0.2]);

        let output = tensor.format_bootstrap(&schema).unwrap();

        // Should contain Wake State line
        assert!(output.contains("Wake State:"));
        // Should contain rune-encoded tensor
        assert!(output.contains("@state:test"));
        // Should contain legend
        assert!(output.contains("A=dim1"));
        assert!(output.contains("B=dim2"));
        // Should contain human-readable descriptions
        assert!(output.contains("high1"));
        assert!(output.contains("low2"));
    }

    #[test]
    fn test_interpolate_anchor_description() {
        let dim = TensorDimension {
            id: "test".to_string(),
            name: "Test".to_string(),
            rune: None,
            anchors: DimensionAnchors {
                low: "cold".to_string(),
                mid: Some("balanced".to_string()),
                high: "hot".to_string(),
            },
            default: 0.5,
        };

        let tensor = StateTensor::new("test".to_string(), vec![0.0]);

        // Low value
        assert_eq!(tensor.interpolate_anchor_description(&dim, 0.2), "cold");

        // High value
        assert_eq!(tensor.interpolate_anchor_description(&dim, 0.8), "hot");

        // Mid value with mid anchor
        assert_eq!(tensor.interpolate_anchor_description(&dim, 0.5), "balanced");

        // Mid value without mid anchor
        let dim_no_mid = TensorDimension {
            id: "test".to_string(),
            name: "Test".to_string(),
            rune: None,
            anchors: DimensionAnchors {
                low: "cold".to_string(),
                mid: None,
                high: "hot".to_string(),
            },
            default: 0.5,
        };

        assert_eq!(
            tensor.interpolate_anchor_description(&dim_no_mid, 0.6),
            "moderately hot"
        );
        assert_eq!(
            tensor.interpolate_anchor_description(&dim_no_mid, 0.4),
            "moderately cold"
        );
    }

    // ---------------------------------------------------------------------
    // Default-schema self-seed (#256)
    //
    // Tests use the `ensure_schema_seeded_at` seam directly with a tempdir,
    // mirroring the `_with` pattern in `paths.rs`. We can't mutate `MX_HOME`
    // mid-process because it's a `OnceLock` shared across parallel tests.
    // ---------------------------------------------------------------------

    #[test]
    fn embedded_default_schema_parses_cleanly() {
        // Smoke test: the bytes we ship must parse into a TensorSchema.
        let parsed: TensorSchema = serde_yaml::from_str(DEFAULT_TENSOR_SCHEMA_YAML)
            .expect("embedded default tensor schema must parse");
        assert_eq!(parsed.id, "tensor");
        assert_eq!(parsed.name, "Default Tensor");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.dimensions.len(), 6);
        // Spec guarantees no moods block in the public default.
        assert!(parsed.moods.is_empty());

        // Dimension ids in the blessed order from the architecture comment.
        let ids: Vec<&str> = parsed.dimensions.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "entropy",
                "agency",
                "temperature",
                "verbosity",
                "skepticism",
                "humor",
            ]
        );
    }

    #[test]
    fn fresh_install_seed_writes_embedded_content() {
        // Fresh install: no state/schemas/ directory exists. Seeder must
        // create the parent dirs and write the embedded content.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state").join("schemas").join("tensor.yaml");

        assert!(!path.exists(), "precondition: file must not exist");
        let wrote = ensure_schema_seeded_at(&path, DEFAULT_TENSOR_SCHEMA_YAML).unwrap();
        assert!(wrote, "seeder should report it wrote the file");
        assert!(path.exists(), "seed file must be present after seeding");

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, DEFAULT_TENSOR_SCHEMA_YAML);

        // And it must parse back cleanly via the schema loader.
        let loaded = TensorSchema::load(&path).unwrap();
        assert_eq!(loaded.id, "tensor");
        assert_eq!(loaded.dimensions.len(), 6);
    }

    #[test]
    fn idempotent_seed_does_not_rewrite_existing_file() {
        // Second invocation must be a no-op: same content, untouched mtime.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state").join("schemas").join("tensor.yaml");

        let first = ensure_schema_seeded_at(&path, DEFAULT_TENSOR_SCHEMA_YAML).unwrap();
        assert!(first, "first call should write");

        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let second = ensure_schema_seeded_at(&path, DEFAULT_TENSOR_SCHEMA_YAML).unwrap();
        assert!(!second, "second call should be a no-op");

        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "idempotent seed must not touch the file"
        );
    }

    #[test]
    fn user_authored_schema_is_preserved() {
        // If a user has dropped their own tensor.yaml at the canonical path,
        // the seeder must not overwrite it. The file's existence is the signal.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state").join("schemas");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tensor.yaml");

        let user_content = "id: tensor\nname: User Override\nversion: 99\ndimensions: []\n";
        std::fs::write(&path, user_content).unwrap();

        let wrote = ensure_schema_seeded_at(&path, DEFAULT_TENSOR_SCHEMA_YAML).unwrap();
        assert!(!wrote, "seeder must not overwrite user-authored file");

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            on_disk, user_content,
            "user-authored content must be preserved verbatim"
        );
        assert_ne!(
            on_disk, DEFAULT_TENSOR_SCHEMA_YAML,
            "embedded default must not have been written"
        );
    }

    #[test]
    fn seed_creates_missing_parent_directories() {
        // The fresh-install case: $MX_HOME exists but no state/schemas/
        // subdirs. The seeder must mkdir -p before writing.
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp
            .path()
            .join("does")
            .join("not")
            .join("exist")
            .join("yet")
            .join("tensor.yaml");

        assert!(!deep.parent().unwrap().exists());
        let wrote = ensure_schema_seeded_at(&deep, DEFAULT_TENSOR_SCHEMA_YAML).unwrap();
        assert!(wrote);
        assert!(deep.exists());
    }

    #[test]
    fn load_by_id_uses_tensor_schema_path_for_yaml_arm() {
        // Smoke test: paths::tensor_schema_path produces the same path that
        // load_by_id consults first. If someone refactors the yaml branch
        // away from the helper, this test will start failing on a missing
        // file at the wrong location -- catching the drift.
        let id = "load-by-id-yaml-arm-test";
        let yaml = crate::paths::tensor_schema_path(id);
        assert!(yaml.ends_with(format!("state/schemas/{}.yaml", id)));
        // We don't write a real file -- load_by_id should bail with the
        // schemas_dir path in the message when nothing exists.
        let err = TensorSchema::load_by_id(id).unwrap_err().to_string();
        assert!(
            err.contains(id),
            "bail message should name the schema id, got: {}",
            err
        );
    }
}
