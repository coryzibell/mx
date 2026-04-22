//! Emotional State Tensor - Schema-driven state encoding/decoding
//!
//! Implements the Emotional State Tensor system designed by Schemnya.
//! States are encoded as stele strings for token efficiency, with full
//! schema-driven validation and backward compatibility with discrete modes.

mod legacy;
mod loader;
mod schema;

// Re-export the full public API to preserve `crate::state::Type` paths.
#[allow(unused_imports)]
pub use legacy::{EmotionalState, TowardState, interactive_capture, parse_wake_preference};
pub use loader::{load_default_schema, load_schema, parse_wake_preference_dynamic};
#[allow(unused_imports)]
pub use schema::{
    Dimension, DimensionHints, DynamicState, ModeMapping, StateSchema, StateValue, SteleConfig,
};

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
            "name": "test",
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
            "name": "test",
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
    use std::collections::HashMap;

    fn get_q_schema() -> StateSchema {
        let schema_json = include_str!("../../schemas/example-q-state.json");
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
