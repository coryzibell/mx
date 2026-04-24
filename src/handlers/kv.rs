//! Handler for `mx kv` subcommands. Wires CLI to the KV engine.

use anyhow::Result;

use crate::cli::{DumpFormat, KvCommands};
use crate::kv::{self, KvError, KvStore};

/// Map a KvError to the appropriate exit code.
fn exit_code_for(err: &KvError) -> Option<i32> {
    match err {
        KvError::KeyNotFound(_) => Some(kv::EXIT_KEY_NOT_FOUND),
        KvError::TypeMismatch { .. } => Some(kv::EXIT_TYPE_MISMATCH),
        KvError::SchemaMissing(_) => Some(kv::EXIT_SCHEMA_MISSING),
        KvError::Other(_) => None,
    }
}

/// Handle a KvError: print to stderr and return exit code, or propagate as anyhow.
fn handle_kv_err(err: KvError) -> Result<i32> {
    match exit_code_for(&err) {
        Some(code) => {
            eprintln!("{}", err);
            Ok(code)
        }
        None => match err {
            KvError::Other(e) => Err(e),
            _ => unreachable!(),
        },
    }
}

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
            Err(e) => handle_kv_err(e),
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
                Err(e) => handle_kv_err(e),
            }
        }

        KvCommands::Inc { key, by } => match store.inc(&key, by) {
            Ok(val) => {
                store.save()?;
                println!("{}", val);
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Dec { key, by } => match store.dec(&key, by) {
            Ok(val) => {
                store.save()?;
                println!("{}", val);
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Push { key, value } => match store.push(&key, &value) {
            Ok(()) => {
                store.save()?;
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Pop { key } => match store.pop(&key) {
            Ok(Some(entry)) => {
                store.save()?;
                if entry.ts.is_empty() {
                    println!("{}: {}", entry.id, entry.value);
                } else {
                    println!("{}: {} ({})", entry.id, entry.value, entry.ts);
                }
                Ok(kv::EXIT_OK)
            }
            Ok(None) => {
                // Nothing was popped — skip save, nothing changed
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Last { key, count } => match store.last(&key, count) {
            Ok(items) => {
                for item in &items {
                    println!("{}", item);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Since { key, timeref } => match store.since(&key, &timeref) {
            Ok(entries) => {
                for entry in &entries {
                    println!("{}: {} ({})", entry.id, entry.value, entry.ts);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Dump { format } => {
            match format {
                DumpFormat::Compact => {
                    println!("{}", store.dump_compact());
                }
                DumpFormat::Json => {
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
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Remove {
            key,
            value,
            id,
            all,
        } => {
            // Must have either value or id
            if value.is_none() && id.is_none() {
                eprintln!("Error: provide either a value substring or --id");
                return Ok(kv::EXIT_KEY_NOT_FOUND);
            }
            match store.remove(&key, value.as_deref(), id, all) {
                Ok(result) => {
                    if result.removed.is_empty() {
                        eprintln!("No matching entries found");
                        Ok(kv::EXIT_KEY_NOT_FOUND)
                    } else {
                        for val in &result.removed {
                            println!("Removed: {}", val);
                        }
                        store.save()?;
                        Ok(kv::EXIT_OK)
                    }
                }
                Err(e) => handle_kv_err(e),
            }
        }

        KvCommands::Search { key, query } => match store.search(&key, &query) {
            Ok(hits) => {
                if hits.is_empty() {
                    eprintln!("No matching entries");
                    Ok(kv::EXIT_OK)
                } else {
                    for hit in &hits {
                        if hit.ts.is_empty() {
                            println!("{}: {}", hit.id, hit.value);
                        } else {
                            println!("{}: {} ({})", hit.id, hit.value, hit.ts);
                        }
                    }
                    Ok(kv::EXIT_OK)
                }
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Count { key, value } => match store.count(&key, value.as_deref()) {
            Ok(result) => {
                match result.latest_ts {
                    Some(ts) => println!("{} (latest: {})", result.total, ts),
                    None => println!("{}", result.total),
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
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
