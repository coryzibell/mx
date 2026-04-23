//! Handler for `mx kv` subcommands. Wires CLI to the KV engine.

use anyhow::Result;

use crate::cli::KvCommands;
use crate::kv::{self, KvStore};

/// Handle all `mx kv` subcommands. Returns the exit code directly.
pub(crate) fn handle_kv(cmd: KvCommands) -> Result<i32> {
    let mut store = match KvStore::from_env() {
        Ok(s) => s,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Failed to read schema") || msg.contains("No such file") {
                eprintln!("Error: schema file not found. {}", msg);
                return Ok(kv::EXIT_SCHEMA_MISSING);
            }
            return Err(e);
        }
    };

    match cmd {
        KvCommands::Get { key } => match store.get(&key) {
            Ok(val) => {
                println!("{}", kv::format_value(val));
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("has no data yet") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) => Err(e),
        },

        KvCommands::Set {
            key,
            value,
            field_value,
        } => {
            // For state types: mx kv set <key> <field> <value>
            // value = field name, field_value = actual value
            // For string/counter: mx kv set <key> <value>
            let result = if let Some(fv) = &field_value {
                store.set(&key, fv, Some(&value))
            } else {
                store.set(&key, &value, None)
            };

            match result {
                Ok(()) => {
                    store.save()?;
                    Ok(kv::EXIT_OK)
                }
                Err(e) if e.to_string().contains("Unknown key") => {
                    eprintln!("{}", e);
                    Ok(kv::EXIT_KEY_NOT_FOUND)
                }
                Err(e) if e.to_string().contains("Type mismatch") => {
                    eprintln!("{}", e);
                    Ok(kv::EXIT_TYPE_MISMATCH)
                }
                Err(e) => Err(e),
            }
        }

        KvCommands::Inc { key, by } => match store.inc(&key, by) {
            Ok(val) => {
                store.save()?;
                println!("{}", val);
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Dec { key, by } => match store.dec(&key, by) {
            Ok(val) => {
                store.save()?;
                println!("{}", val);
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Push { key, value } => match store.push(&key, &value) {
            Ok(()) => {
                store.save()?;
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Pop { key } => match store.pop(&key) {
            Ok(Some(val)) => {
                store.save()?;
                println!("{}", val);
                Ok(kv::EXIT_OK)
            }
            Ok(None) => {
                store.save()?;
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Last { key, count } => match store.last(&key, count) {
            Ok(items) => {
                for item in &items {
                    println!("{}", item);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Since { key, timeref } => match store.since(&key, &timeref) {
            Ok(entries) => {
                for entry in &entries {
                    println!("{} ({})", entry.value, entry.ts);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) if e.to_string().contains("Type mismatch") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_TYPE_MISMATCH)
            }
            Err(e) => Err(e),
        },

        KvCommands::Dump { format } => {
            match format.as_str() {
                "compact" => {
                    println!("{}", store.dump_compact());
                }
                _ => {
                    println!("{}", store.dump_json()?);
                }
            }
            Ok(kv::EXIT_OK)
        }

        KvCommands::Reset { key } => match store.reset(&key) {
            Ok(()) => {
                store.save()?;
                Ok(kv::EXIT_OK)
            }
            Err(e) if e.to_string().contains("Unknown key") => {
                eprintln!("{}", e);
                Ok(kv::EXIT_KEY_NOT_FOUND)
            }
            Err(e) => Err(e),
        },

        KvCommands::Keys => {
            let keys = store.keys();
            for (name, vtype) in &keys {
                println!("{:30} {}", name, vtype);
            }
            Ok(kv::EXIT_OK)
        }
    }
}
