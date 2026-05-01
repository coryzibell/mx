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

/// Resolve and display a memory pointer for a key.
/// Connects to SurrealDB, fetches the kn- entry, and prints it.
/// Failures are non-fatal: KV data is primary.
fn resolve_memory(store: &KvStore, key: &str, verbose: bool) {
    let mem = match store.get_memory(key) {
        Ok(Some(m)) => m.to_string(),
        Ok(None) => return,
        Err(_) => return, // type mismatch or not found — silently skip
    };

    print_resolved_memory(&mem, verbose);
}

/// Fetch and display a single memory entry by kn- ID.
fn print_resolved_memory(kn_id: &str, verbose: bool) {
    use crate::index::IndexConfig;
    use crate::store::{self, AgentContext};

    let config = IndexConfig::default();
    let db = match store::create_store_with_verbose(&config.db_path, verbose) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Warning: could not connect to memory store: {}", e);
            return;
        }
    };

    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => AgentContext::for_agent(agent),
        _ => AgentContext::public_only(),
    };

    match db.get(kn_id, &ctx) {
        Ok(Some(entry)) => {
            println!();
            println!("Memory ({}):", kn_id);
            println!("  Title:    {}", entry.title);
            println!("  Category: {}", entry.category_id);
            if let Some(body) = &entry.body {
                // Indent body content
                for line in body.lines() {
                    println!("  {}", line);
                }
            }
        }
        Ok(None) => {
            eprintln!("Warning: memory entry {} not found", kn_id);
        }
        Err(e) => {
            eprintln!("Warning: failed to fetch memory entry {}: {}", kn_id, e);
        }
    }
}

/// Resolve memory pointers for all keys in a dump.
fn resolve_dump_memories(store: &KvStore, verbose: bool) {
    for (key, _vtype) in store.keys() {
        if let Ok(Some(mem)) = store.get_memory(key) {
            println!();
            println!("--- {} ---", key);
            print_resolved_memory(mem, verbose);
        }
    }
}

/// Handle all `mx kv` subcommands. Returns the exit code directly.
pub(crate) fn handle_kv(cmd: KvCommands, verbose: bool) -> Result<i32> {
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
        KvCommands::Get { key, memory } => match store.get(&key) {
            Ok(val) => {
                println!("{}", kv::format_value(val));
                if memory {
                    resolve_memory(&store, &key, verbose);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Set {
            key,
            value,
            field_value,
            memory,
        } => {
            let mut did_something = false;

            // Handle the value set (if a value was provided)
            if let Some(ref val) = value {
                // For state types: mx kv set <key> <field> <value>
                // value = field name, field_value = actual value
                // For string/counter: mx kv set <key> <value>
                let result = if let Some(fv) = &field_value {
                    store.set(&key, fv, Some(val))
                } else {
                    store.set(&key, val, None)
                };

                match result {
                    Ok(()) => {
                        did_something = true;
                    }
                    Err(e) => {
                        // If --memory was also provided, still try the memory set
                        // only if the type mismatch is because it's a history/list
                        // (set doesn't normally work on those). Otherwise, propagate error.
                        if memory.is_none() {
                            return handle_kv_err(e);
                        }
                        // Type mismatch on history/list is expected when only --memory is provided
                        match &e {
                            KvError::TypeMismatch { .. } => {
                                // Allow falling through to memory-only set
                                eprintln!(
                                    "Warning: value not set (type does not support set); memory pointer updated"
                                );
                            }
                            _ => return handle_kv_err(e),
                        }
                    }
                }
            }

            // Handle the memory pointer (if --memory was provided)
            if let Some(mem_val) = memory {
                let mem = if mem_val.is_empty() {
                    None
                } else {
                    Some(mem_val)
                };
                match store.set_memory(&key, mem) {
                    Ok(()) => {
                        did_something = true;
                    }
                    Err(e) => return handle_kv_err(e),
                }
            }

            if !did_something {
                // Neither value nor memory was set — need at least one
                eprintln!("Error: provide a value or --memory");
                return Ok(kv::EXIT_KEY_NOT_FOUND);
            }

            store.save()?;
            Ok(kv::EXIT_OK)
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

        KvCommands::Last { key, count, memory } => match store.last(&key, count) {
            Ok(items) => {
                for item in &items {
                    println!("{}", item);
                }
                if memory {
                    resolve_memory(&store, &key, verbose);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Since {
            key,
            timeref,
            memory,
        } => match store.since(&key, &timeref) {
            Ok(entries) => {
                for entry in &entries {
                    println!("{}: {} ({})", entry.id, entry.value, entry.ts);
                }
                if memory {
                    resolve_memory(&store, &key, verbose);
                }
                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Dump { format, memory } => {
            match format {
                DumpFormat::Compact => {
                    println!("{}", store.dump_compact());
                }
                DumpFormat::Json => {
                    println!("{}", store.dump_json()?);
                }
            }
            if memory {
                resolve_dump_memories(&store, verbose);
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

        KvCommands::Search { key, query, memory } => match store.search(&key, &query) {
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
                    if memory {
                        resolve_memory(&store, &key, verbose);
                    }
                    Ok(kv::EXIT_OK)
                }
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Count { key, value } => match store.count(&key, value.as_deref()) {
            Ok(result) => {
                match result.total {
                    Some(total) => {
                        // Filtered: show matched/total (pct%) — latest: ...
                        let pct = if total == 0 {
                            0
                        } else {
                            ((result.matched as f64 / total as f64) * 100.0).round() as u64
                        };
                        match result.latest_ts {
                            Some(ts) => println!(
                                "{}/{} ({}%) \u{2014} latest: {}",
                                result.matched, total, pct, ts
                            ),
                            None => println!("{}/{} ({}%)", result.matched, total, pct),
                        }
                    }
                    None => {
                        // Unfiltered: preserve original format
                        match result.latest_ts {
                            Some(ts) => println!("{} (latest: {})", result.matched, ts),
                            None => println!("{}", result.matched),
                        }
                    }
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
