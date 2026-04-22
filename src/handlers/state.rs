use anyhow::{Context, Result, bail};

use crate::cli::*;
use crate::state;
use crate::tensor;

/// Handle emotional state tensor commands
pub(crate) fn handle_state(cmd: StateCommands) -> Result<()> {
    use std::io::{self, Read as IoRead};
    use std::path::PathBuf;

    // Helper to load tensor schema by ID or path
    let load_tensor_schema = |schema_arg: Option<String>| -> Result<tensor::TensorSchema> {
        match schema_arg {
            Some(s) if s.contains('/') || s.contains('.') => {
                // Looks like a path
                tensor::TensorSchema::load(&PathBuf::from(s))
            }
            Some(id) => tensor::TensorSchema::load_by_id(&id),
            None => tensor::TensorSchema::load_default(),
        }
    };

    // Helper to load legacy state schema
    let load_legacy_schema = |custom_path: Option<String>| -> Result<state::StateSchema> {
        match custom_path {
            Some(p) => state::load_schema(&PathBuf::from(p)),
            None => state::load_default_schema(),
        }
    };

    match cmd {
        // === NEW TENSOR-BASED COMMANDS ===
        StateCommands::Encode {
            values,
            dimensions,
            file,
            schema,
            guided,
            format,
            runes,
        } => {
            let schema = load_tensor_schema(schema)?;

            let tensor = if guided {
                // Interactive guided mode
                tensor::guided_capture(&schema)?
            } else if let Some(dims_str) = dimensions {
                // Parse named dimensions
                tensor::StateTensor::parse_named_dimensions(&schema, &dims_str)?
            } else if let Some(file_path) = file {
                // Read from file
                let content = std::fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?;

                // Try pipe-separated first, then newline-separated
                let values_str = if content.contains('|') {
                    content.trim().to_string()
                } else {
                    content
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join("|")
                };

                tensor::StateTensor::parse_values(&schema, &values_str)?
            } else if let Some(values_str) = values {
                // Parse from argument
                tensor::StateTensor::parse_values(&schema, &values_str)?
            } else {
                // Default tensor
                tensor::StateTensor::default_from_schema(&schema)
            };

            // Output in requested format
            match format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&tensor)?),
                "human" => {
                    println!("{}", tensor.describe(&schema));
                    if let Some((mood_name, mood, distance)) = tensor.nearest_mood(&schema) {
                        println!("\nNearest mood: {} (distance: {:.3})", mood_name, distance);
                        println!("  {}", mood.description);
                    }
                }
                "bootstrap" => {
                    // Self-documenting bootstrap format
                    println!("{}", tensor.format_bootstrap(&schema)?);
                }
                _ => {
                    // tensor format
                    if runes {
                        println!("{}", tensor.encode_with_runes(&schema));
                    } else {
                        println!("{}", tensor.encode());
                    }
                }
            }
        }

        StateCommands::Decode {
            input,
            schema,
            format,
        } => {
            // Get input from arg or stdin
            let input_str = match input {
                Some(s) => s,
                None => {
                    let mut buf = String::new();
                    io::stdin().read_to_string(&mut buf)?;
                    buf.trim().to_string()
                }
            };

            // Decode the tensor (schema ID is embedded in the string)
            let tensor = tensor::StateTensor::decode(&input_str)?;

            // Load schema (use argument if provided, otherwise use ID from tensor)
            let schema = match schema {
                Some(s) => load_tensor_schema(Some(s))?,
                None => tensor::TensorSchema::load_by_id(&tensor.schema_id)?,
            };

            match format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&tensor)?),
                "tensor" => println!("{}", tensor.encode()),
                "mood" => {
                    if let Some((mood_name, mood, distance)) = tensor.nearest_mood(&schema) {
                        println!("{}", mood_name);
                        println!("  Description: {}", mood.description);
                        println!("  Distance: {:.3}", distance);
                    } else {
                        println!("(unnamed region)");
                    }
                }
                _ => {
                    // human format
                    println!("{}", tensor.describe(&schema));
                    if let Some((mood_name, mood, distance)) = tensor.nearest_mood(&schema) {
                        println!("\nNearest mood: {} (distance: {:.3})", mood_name, distance);
                        println!("  {}", mood.description);
                    }
                }
            }
        }

        StateCommands::Schemas { json } => {
            let schemas = tensor::TensorSchema::list_available()?;

            if json {
                let schema_list: Vec<serde_json::Value> = schemas
                    .iter()
                    .filter_map(|schema_id| {
                        tensor::TensorSchema::load_by_id(schema_id).ok().map(|s| {
                            serde_json::json!({
                                "id": s.id,
                                "name": s.name,
                                "dimensions": s.dimensions.len(),
                                "moods": s.moods.len(),
                            })
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&schema_list)?);
            } else if schemas.is_empty() {
                println!("No schemas found (checked $MX_HOME/schemas/)");
                println!("\nCreate a schema file (YAML or JSON) to get started.");
            } else {
                println!("Available schemas:\n");
                for schema_id in schemas {
                    match tensor::TensorSchema::load_by_id(&schema_id) {
                        Ok(schema) => {
                            println!(
                                "  {} - {} ({} dimensions, {} moods)",
                                schema.id,
                                schema.name,
                                schema.dimensions.len(),
                                schema.moods.len()
                            );
                        }
                        Err(_) => {
                            println!("  {} - (failed to load)", schema_id);
                        }
                    }
                }
            }
        }

        StateCommands::Moods { schema, mood, json } => {
            let schema = load_tensor_schema(schema)?;

            if let Some(mood_name) = mood {
                // Show specific mood
                match schema.moods.get(&mood_name) {
                    Some(mood_def) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&mood_def)?);
                        } else {
                            println!("Mood: {}", mood_name);
                            println!("Description: {}", mood_def.description);
                            println!("Tolerance: {:.2}", mood_def.tolerance);
                            println!("\nTensor values:");
                            for (i, value) in mood_def.tensor.iter().enumerate() {
                                let dim_name = schema
                                    .dimensions
                                    .get(i)
                                    .map(|d| d.name.as_str())
                                    .unwrap_or("?");
                                let weight = mood_def
                                    .weights
                                    .as_ref()
                                    .and_then(|w| w.get(i))
                                    .copied()
                                    .unwrap_or(1.0);
                                println!("  {}: {:.2} (weight: {:.2})", dim_name, value, weight);
                            }
                        }
                    }
                    None => {
                        bail!(
                            "Unknown mood '{}'. Available moods: {}",
                            mood_name,
                            schema.moods.keys().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                }
            } else {
                // List all moods
                if json {
                    println!("{}", serde_json::to_string_pretty(&schema.moods)?);
                } else {
                    println!("Moods for schema '{}' ({}):\n", schema.id, schema.name);
                    for (name, mood_def) in &schema.moods {
                        let tensor_str: Vec<String> = mood_def
                            .tensor
                            .iter()
                            .map(|v| format!("{:.2}", v))
                            .collect();
                        println!("  {:12} [{}]", name, tensor_str.join("|"));
                        println!("               {}", mood_def.description);
                    }
                }
            }
        }

        StateCommands::Info { schema, json } => {
            let schema = load_tensor_schema(schema)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&schema)?);
            } else {
                println!("Schema: {} ({})", schema.name, schema.id);
                println!("Version: {}", schema.version);
                println!();
                println!("Dimensions ({}):", schema.dimensions.len());
                for dim in &schema.dimensions {
                    let rune = dim
                        .rune
                        .as_ref()
                        .map(|r| format!(" {}", r))
                        .unwrap_or_default();
                    println!("  {}{}:", dim.name, rune);
                    println!("    Low:  {}", dim.anchors.low);
                    if let Some(mid) = &dim.anchors.mid {
                        println!("    Mid:  {}", mid);
                    }
                    println!("    High: {}", dim.anchors.high);
                    println!("    Default: {:.2}", dim.default);
                }
                println!();
                println!("Moods ({}):", schema.moods.len());
                for (name, mood) in &schema.moods {
                    println!(
                        "  {:12} - {} (tol: {:.2})",
                        name, mood.description, mood.tolerance
                    );
                }
            }
        }

        // === LEGACY COMMANDS (backward compatibility) ===
        StateCommands::LegacyEncode {
            mode,
            interactive,
            format,
            schema,
        } => {
            let schema = load_legacy_schema(schema)?;

            let dynamic_state = if interactive {
                state::DynamicState::interactive_capture(&schema)?
            } else if let Some(mode_name) = mode {
                state::DynamicState::from_mode(&mode_name, &schema)?
            } else {
                state::DynamicState::from_mode("default", &schema)?
            };

            match format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&dynamic_state)?),
                "human" => println!("{}", dynamic_state.describe(&schema)),
                _ => println!("{}", dynamic_state.encode_stele(&schema)),
            }
        }

        StateCommands::Parse {
            file,
            preference,
            format,
            schema,
        } => {
            // Strip leading markdown bold markers (**/__) so "**Wake State:** ..."
            // matches the same as plain "Wake State: ...".  Only used in the predicate;
            // the original line is kept for value extraction.
            fn strip_md_bold(line: &str) -> &str {
                line.trim_start()
                    .trim_start_matches("**")
                    .trim_start_matches("__")
                    .trim_start()
            }

            // Extract the @state:... fragment from a matched line, stripping any
            // leading label ("Wake State:", "**Wake State:**", etc.) and trailing
            // markdown bold close ("**") that pocket inserts around the label.
            fn extract_stele_fragment(line: &str) -> &str {
                // Find the @state token wherever it appears in the line
                if let Some(pos) = line.find("@state") {
                    line[pos..]
                        .trim_end_matches('*')
                        .trim_end_matches('_')
                        .trim()
                } else {
                    line.trim()
                }
            }

            let legacy_schema = load_legacy_schema(schema.clone())?;

            let raw_line = if let Some(pref) = preference {
                pref
            } else {
                let path = file.unwrap_or_else(|| {
                    crate::paths::swap_dir()
                        .join("session-bootstrap.md")
                        .to_string_lossy()
                        .to_string()
                });

                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read file: {}", path))?;

                content
                    .lines()
                    .find(|line| {
                        let stripped = strip_md_bold(line);
                        stripped.starts_with("Wake Preference:")
                            || stripped.starts_with("Wake State:")
                            || stripped.starts_with(&legacy_schema.stele.header)
                    })
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| String::from("default"))
            };

            // Detect tensor format: @state:<namespace>|... (has colon after @state)
            let stele_fragment = extract_stele_fragment(&raw_line);
            let is_tensor_format =
                stele_fragment.starts_with("@state:") && stele_fragment.contains('|');

            if is_tensor_format {
                // Decode via tensor path which handles positional numeric values
                let tensor_schema = load_tensor_schema(schema)?;
                let tensor = tensor::StateTensor::decode(stele_fragment).with_context(|| {
                    format!("Failed to decode tensor stele: {}", stele_fragment)
                })?;

                match format.as_str() {
                    "json" => println!("{}", serde_json::to_string_pretty(&tensor.values)?),
                    "stele" => println!("{}", tensor.encode()),
                    _ => {
                        println!("Parsed: {}", stele_fragment);
                        println!();
                        println!("{}", tensor.describe(&tensor_schema));
                    }
                }
            } else {
                // Legacy path: mode names or rune-prefixed stele
                let dynamic_state =
                    state::parse_wake_preference_dynamic(&raw_line, &legacy_schema)?;

                match format.as_str() {
                    "json" => println!("{}", serde_json::to_string_pretty(&dynamic_state)?),
                    "stele" => println!("{}", dynamic_state.encode_stele(&legacy_schema)),
                    "mode" => {
                        println!("Mode calculation not yet implemented for DynamicState");
                    }
                    _ => {
                        println!("Parsed: {}", raw_line.trim());
                        println!();
                        println!("{}", dynamic_state.describe(&legacy_schema));
                    }
                }
            }
        }
    }

    Ok(())
}
