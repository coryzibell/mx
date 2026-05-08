//! Handler for `mx kv` subcommands. Wires CLI to the KV engine.

use std::collections::HashSet;

use anyhow::Result;

use crate::cli::{DumpFormat, KvCommands};
use crate::kv::{self, KvError, KvStore, resolve_time_range};

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

/// Parse an ID specification into a sorted, deduplicated list of u64 IDs.
///
/// Accepted formats:
/// - Single ID: "35" -> [35]
/// - Range: "35-64" -> [35, 36, ..., 64]
/// - Comma-separated: "1,5,12" -> [1, 5, 12]
fn parse_id_spec(spec: &str) -> Result<Vec<u64>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty ID specification".to_string());
    }

    let mut ids: Vec<u64> = if spec.contains(',') {
        // Comma-separated list
        spec.split(',')
            .map(|s| {
                s.trim()
                    .parse::<u64>()
                    .map_err(|_| format!("invalid ID '{}' in spec '{}'", s.trim(), spec))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else if spec.contains('-') {
        // Range
        let parts: Vec<&str> = spec.splitn(2, '-').collect();
        let start: u64 = parts[0].trim().parse().map_err(|_| {
            format!(
                "invalid range start '{}' in spec '{}'",
                parts[0].trim(),
                spec
            )
        })?;
        let end: u64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| format!("invalid range end '{}' in spec '{}'", parts[1].trim(), spec))?;
        if start > end {
            return Err(format!(
                "invalid range: start ({}) is greater than end ({})",
                start, end
            ));
        }
        const MAX_RANGE_SIZE: u64 = 10_000;
        if end - start + 1 > MAX_RANGE_SIZE {
            return Err(format!(
                "range too large ({} entries, max {})",
                end - start + 1,
                MAX_RANGE_SIZE
            ));
        }
        (start..=end).collect()
    } else {
        // Single ID
        let id: u64 = spec.parse().map_err(|_| format!("invalid ID '{}'", spec))?;
        vec![id]
    };

    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
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
        KvCommands::Get { key, id, memory } => {
            if let Some(id_spec) = id {
                // ID-based entry lookup on history/list
                let ids = match parse_id_spec(&id_spec) {
                    Ok(ids) => ids,
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                };
                match store.get_entries_by_id(&key, &ids) {
                    Ok(hits) => {
                        // Print found entries.
                        // History entries always have a timestamp; list entries may not.
                        // We check for empty ts to handle both types uniformly.
                        for hit in &hits {
                            if hit.ts.is_empty() {
                                println!("{}: {}", hit.id, hit.value);
                            } else {
                                println!("{}: {} ({})", hit.id, hit.value, hit.ts);
                            }
                        }

                        // Report missing IDs
                        let found_ids: HashSet<u64> = hits.iter().map(|h| h.id).collect();
                        let missing: Vec<u64> = ids
                            .iter()
                            .filter(|id| !found_ids.contains(id))
                            .copied()
                            .collect();
                        if !missing.is_empty() {
                            let missing_str: Vec<String> =
                                missing.iter().map(|id| id.to_string()).collect();
                            eprintln!("note: IDs not found: {}", missing_str.join(", "));
                        }

                        if memory {
                            // Resolve memory for each entry value that looks like a kn- reference
                            for hit in &hits {
                                if hit.value.starts_with("kn-") {
                                    print_resolved_memory(&hit.value, verbose);
                                }
                            }
                            // Also resolve key-level memory pointer
                            resolve_memory(&store, &key, verbose);
                        }

                        Ok(kv::EXIT_OK)
                    }
                    Err(e) => handle_kv_err(e),
                }
            } else {
                // Original scalar get behavior
                match store.get(&key) {
                    Ok(val) => {
                        println!("{}", kv::format_value(val));
                        if memory {
                            resolve_memory(&store, &key, verbose);
                        }
                        Ok(kv::EXIT_OK)
                    }
                    Err(e) => handle_kv_err(e),
                }
            }
        }

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

        KvCommands::Last {
            key,
            count,
            memory,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            match store.last(&key, count, range.as_ref()) {
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
            }
        }

        KvCommands::Random {
            key,
            count,
            memory,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            match store.random(&key, count, range.as_ref()) {
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
            }
        }

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

        KvCommands::Search {
            key,
            query,
            memory,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            match store.search(&key, &query, range.as_ref()) {
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
            }
        }

        KvCommands::Count {
            key,
            value,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            match store.count(&key, value.as_deref(), range.as_ref()) {
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
            }
        }

        KvCommands::Keys => {
            let keys = store.keys();
            for (name, vtype) in &keys {
                println!("{:30} {}", name, vtype);
            }
            Ok(kv::EXIT_OK)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_id_spec --

    #[test]
    fn parse_single_id() {
        assert_eq!(parse_id_spec("35").unwrap(), vec![35]);
    }

    #[test]
    fn parse_single_id_zero() {
        assert_eq!(parse_id_spec("0").unwrap(), vec![0]);
    }

    #[test]
    fn parse_range() {
        assert_eq!(parse_id_spec("3-7").unwrap(), vec![3, 4, 5, 6, 7]);
    }

    #[test]
    fn parse_range_single_element() {
        assert_eq!(parse_id_spec("5-5").unwrap(), vec![5]);
    }

    #[test]
    fn parse_range_start_greater_than_end() {
        let result = parse_id_spec("10-5");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("greater than end"));
    }

    #[test]
    fn parse_comma_separated() {
        assert_eq!(parse_id_spec("1,5,12").unwrap(), vec![1, 5, 12]);
    }

    #[test]
    fn parse_comma_separated_deduplicates() {
        assert_eq!(parse_id_spec("5,1,5,3,1").unwrap(), vec![1, 3, 5]);
    }

    #[test]
    fn parse_comma_separated_with_spaces() {
        assert_eq!(parse_id_spec("1, 5, 12").unwrap(), vec![1, 5, 12]);
    }

    #[test]
    fn parse_invalid_single() {
        let result = parse_id_spec("abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid ID"));
    }

    #[test]
    fn parse_invalid_in_list() {
        let result = parse_id_spec("1,5,abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid ID"));
    }

    #[test]
    fn parse_invalid_range_start() {
        let result = parse_id_spec("abc-10");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid range start"));
    }

    #[test]
    fn parse_invalid_range_end() {
        let result = parse_id_spec("1-abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid range end"));
    }

    #[test]
    fn parse_empty_spec() {
        let result = parse_id_spec("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn parse_whitespace_only_spec() {
        let result = parse_id_spec("   ");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn parse_open_ended_range_start() {
        assert!(parse_id_spec("-5").is_err());
    }

    #[test]
    fn parse_open_ended_range_end() {
        assert!(parse_id_spec("5-").is_err());
    }

    #[test]
    fn parse_range_too_large() {
        assert!(parse_id_spec("1-20000").is_err());
    }
}
