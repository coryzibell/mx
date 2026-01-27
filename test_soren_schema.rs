use mx::state::{DynamicState, StateSchema};
use std::collections::HashMap;

fn main() {
    println!("Testing Soren schema with dynamic state operations...\n");

    // Load Soren's schema
    let schema_path = "schemas/example-soren-state.json";
    let schema = StateSchema::from_file(schema_path)
        .expect("Failed to load Soren schema");

    println!("✓ Loaded Soren schema: {}", schema.title);
    println!("  Dimensions: {:?}\n", schema.dimensions.keys().collect::<Vec<_>>());

    // Create a state from mode
    let state = DynamicState::from_mode("tending", &schema)
        .expect("Failed to create state from mode");

    println!("✓ Created state from 'tending' mode");

    // Encode to stele
    let stele = state.encode(&schema)
        .expect("Failed to encode state");

    println!("✓ Encoded to stele:\n  {}\n", stele);

    // Decode from stele
    let decoded = DynamicState::decode(&stele, &schema)
        .expect("Failed to decode stele");

    println!("✓ Decoded from stele");

    // Verify roundtrip
    let values_match = state.values.iter().all(|(k, v)| {
        decoded.values.get(k).map(|dv| {
            match (v, dv) {
                (mx::state::StateValue::Float(a), mx::state::StateValue::Float(b)) => {
                    (a - b).abs() < 0.001
                }
                (mx::state::StateValue::Nested(a), mx::state::StateValue::Nested(b)) => {
                    a.iter().all(|(nk, nv)| {
                        b.get(nk).map(|ndv| match (nv, ndv) {
                            (mx::state::StateValue::Float(na), mx::state::StateValue::Float(nb)) => {
                                (na - nb).abs() < 0.001
                            }
                            _ => false
                        }).unwrap_or(false)
                    })
                }
                _ => false
            }
        }).unwrap_or(false)
    });

    if values_match {
        println!("✓ Roundtrip successful - all values match\n");
    } else {
        println!("✗ Roundtrip failed - values don't match\n");
        std::process::exit(1);
    }

    // Test describe
    let description = state.describe(&schema);
    println!("✓ State description:");
    println!("{}\n", description);

    // Test with different dimensions than Q
    println!("Testing nested dimension 'carrying':");
    if let Some(mx::state::StateValue::Nested(carrying)) = state.values.get("carrying") {
        println!("  threads: {:.2}",
            if let Some(mx::state::StateValue::Float(v)) = carrying.get("threads") { v } else { &0.0 });
        println!("  weight: {:.2}",
            if let Some(mx::state::StateValue::Float(v)) = carrying.get("weight") { v } else { &0.0 });
        println!("  proximity: {:.2}",
            if let Some(mx::state::StateValue::Float(v)) = carrying.get("proximity") { v } else { &0.0 });
    }

    println!("\n✓ All Soren schema tests passed!");
}
