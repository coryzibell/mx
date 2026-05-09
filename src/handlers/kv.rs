//! Handler for `mx kv` subcommands. Wires CLI to the KV engine.

use std::collections::HashSet;

use anyhow::Result;

use crate::cli::{DumpFormat, KvCommands};
use crate::kv::{self, IdRef, KvError, KvStore, resolve_time_range};

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

/// Parse a single token into an `IdRef`.
///
/// - Starts with `kv-` -> strip prefix, treat as hash
/// - Pure digits -> numeric ID
/// - Otherwise -> error
fn parse_single_id(token: &str) -> Result<IdRef, String> {
    let token = token.trim();
    if let Some(hash) = token.strip_prefix("kv-") {
        if hash.is_empty() {
            return Err("empty hash after 'kv-' prefix".to_string());
        }
        Ok(IdRef::Hash(hash.to_string()))
    } else {
        let id: u64 = token
            .parse()
            .map_err(|_| format!("invalid ID '{}'", token))?;
        Ok(IdRef::Numeric(id))
    }
}

/// Parse an ID specification into a list of `IdRef`s.
///
/// Accepted formats:
/// - Single numeric ID: "35" -> [Numeric(35)]
/// - Hash ID: "kv-A3fB" -> [Hash("A3fB")]
/// - Numeric range: "35-64" -> [Numeric(35), ..., Numeric(64)]
/// - Comma-separated (can mix): "1,kv-A3fB,12" -> [Numeric(1), Hash("A3fB"), Numeric(12)]
fn parse_id_spec(spec: &str) -> Result<Vec<IdRef>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty ID specification".to_string());
    }

    if spec.contains(',') {
        // Comma-separated list (can mix numeric and hash)
        spec.split(',')
            .map(|s| parse_single_id(s.trim()))
            .collect::<Result<Vec<_>, _>>()
    } else if spec.starts_with("kv-") {
        // Single hash ID
        Ok(vec![parse_single_id(spec)?])
    } else if spec.contains('-') {
        // Numeric range
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
        Ok((start..=end).map(IdRef::Numeric).collect())
    } else {
        // Single numeric ID
        Ok(vec![parse_single_id(spec)?])
    }
}

/// Parse `--where` clause strings into `(key, value)` tuples.
///
/// Each clause is split on the first `=` character. A clause without `=`
/// returns an error describing the expected format.
fn parse_where_clauses(clauses: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut result = Vec::with_capacity(clauses.len());
    for clause in clauses {
        match clause.split_once('=') {
            Some((k, v)) => result.push((k.to_string(), v.to_string())),
            None => {
                return Err(format!(
                    "invalid --where clause '{}': expected format key=value",
                    clause
                ));
            }
        }
    }
    Ok(result)
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
                        for hit in &hits {
                            println!(
                                "{}",
                                kv::format_entry_line(
                                    hit.id, &hit.hash, &hit.value, &hit.ts, &hit.data
                                )
                            );
                        }

                        // Report missing IDs
                        let found_numeric: HashSet<u64> = hits.iter().map(|h| h.id).collect();
                        let found_hashes: Vec<&str> =
                            hits.iter().map(|h| h.hash.as_str()).collect();
                        let missing: Vec<String> = ids
                            .iter()
                            .filter(|id_ref| match id_ref {
                                IdRef::Numeric(n) => !found_numeric.contains(n),
                                IdRef::Hash(h) => {
                                    !found_hashes.iter().any(|fh| fh.starts_with(h.as_str()))
                                }
                            })
                            .map(|id_ref| match id_ref {
                                IdRef::Numeric(n) => n.to_string(),
                                IdRef::Hash(h) => format!("kv-{}", h),
                            })
                            .collect();
                        if !missing.is_empty() {
                            eprintln!("note: IDs not found: {}", missing.join(", "));
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

        KvCommands::Push { key, value, data } => {
            // Parse --data as JSON object if provided
            let parsed_data = match data {
                Some(ref json_str) => {
                    let val: serde_json::Value = match serde_json::from_str(json_str) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("Error: invalid JSON for --data: {}", e);
                            return Ok(kv::EXIT_INVALID_INPUT);
                        }
                    };
                    if !val.is_object() {
                        eprintln!(
                            "Error: --data must be a JSON object, got {}",
                            match val {
                                serde_json::Value::Array(_) => "array",
                                serde_json::Value::String(_) => "string",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::Bool(_) => "boolean",
                                serde_json::Value::Null => "null",
                                serde_json::Value::Object(_) => unreachable!(),
                            }
                        );
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                    Some(val)
                }
                None => None,
            };

            match store.push(&key, &value, parsed_data) {
                Ok(result) => {
                    store.save()?;
                    println!("kv-{} ({})", result.hash, result.id);
                    Ok(kv::EXIT_OK)
                }
                Err(e) => handle_kv_err(e),
            }
        }

        KvCommands::Pop { key } => match store.pop(&key) {
            Ok(Some(entry)) => {
                store.save()?;
                println!(
                    "{}",
                    kv::format_entry_line(
                        entry.id,
                        &entry.hash,
                        &entry.value,
                        &entry.ts,
                        &entry.data
                    )
                );
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
            where_clauses,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            let parsed_where = match parse_where_clauses(&where_clauses) {
                Ok(w) => w,
                Err(msg) => {
                    eprintln!("Error: {}", msg);
                    return Ok(kv::EXIT_INVALID_INPUT);
                }
            };
            match store.last(&key, count, range.as_ref(), &parsed_where) {
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
            where_clauses,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            let parsed_where = match parse_where_clauses(&where_clauses) {
                Ok(w) => w,
                Err(msg) => {
                    eprintln!("Error: {}", msg);
                    return Ok(kv::EXIT_INVALID_INPUT);
                }
            };
            match store.random(&key, count, range.as_ref(), &parsed_where) {
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
                    println!(
                        "{}",
                        kv::format_entry_line(
                            entry.id,
                            &entry.hash,
                            &entry.value,
                            &entry.ts,
                            &entry.data
                        )
                    );
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
            // Parse --id as an IdRef (numeric or hash)
            let id_ref = match &id {
                Some(id_str) => match parse_single_id(id_str) {
                    Ok(r) => Some(r),
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                },
                None => None,
            };
            match store.remove(&key, value.as_deref(), id_ref.as_ref(), all) {
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
            where_clauses,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            let parsed_where = match parse_where_clauses(&where_clauses) {
                Ok(w) => w,
                Err(msg) => {
                    eprintln!("Error: {}", msg);
                    return Ok(kv::EXIT_INVALID_INPUT);
                }
            };

            // Must have at least a query or where clauses
            if query.is_none() && parsed_where.is_empty() {
                eprintln!("Error: provide a search query or --where filters");
                return Ok(kv::EXIT_INVALID_INPUT);
            }

            match store.search(&key, query.as_deref(), range.as_ref(), &parsed_where) {
                Ok(hits) => {
                    if hits.is_empty() {
                        eprintln!("No matching entries");
                        Ok(kv::EXIT_OK)
                    } else {
                        for hit in &hits {
                            println!(
                                "{}",
                                kv::format_entry_line(
                                    hit.id, &hit.hash, &hit.value, &hit.ts, &hit.data
                                )
                            );
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
            where_clauses,
            time_range,
        } => {
            let range = resolve_time_range(&time_range).map_err(KvError::Other)?;
            let parsed_where = match parse_where_clauses(&where_clauses) {
                Ok(w) => w,
                Err(msg) => {
                    eprintln!("Error: {}", msg);
                    return Ok(kv::EXIT_INVALID_INPUT);
                }
            };
            match store.count(&key, value.as_deref(), range.as_ref(), &parsed_where) {
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
        assert_eq!(parse_id_spec("35").unwrap(), vec![IdRef::Numeric(35)]);
    }

    #[test]
    fn parse_single_id_zero() {
        assert_eq!(parse_id_spec("0").unwrap(), vec![IdRef::Numeric(0)]);
    }

    #[test]
    fn parse_range() {
        assert_eq!(
            parse_id_spec("3-7").unwrap(),
            vec![
                IdRef::Numeric(3),
                IdRef::Numeric(4),
                IdRef::Numeric(5),
                IdRef::Numeric(6),
                IdRef::Numeric(7),
            ]
        );
    }

    #[test]
    fn parse_range_single_element() {
        assert_eq!(parse_id_spec("5-5").unwrap(), vec![IdRef::Numeric(5)]);
    }

    #[test]
    fn parse_range_start_greater_than_end() {
        let result = parse_id_spec("10-5");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("greater than end"));
    }

    #[test]
    fn parse_comma_separated() {
        assert_eq!(
            parse_id_spec("1,5,12").unwrap(),
            vec![IdRef::Numeric(1), IdRef::Numeric(5), IdRef::Numeric(12)]
        );
    }

    #[test]
    fn parse_comma_separated_with_spaces() {
        assert_eq!(
            parse_id_spec("1, 5, 12").unwrap(),
            vec![IdRef::Numeric(1), IdRef::Numeric(5), IdRef::Numeric(12)]
        );
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

    // -- hash ID parsing --

    #[test]
    fn parse_hash_id_single() {
        assert_eq!(
            parse_id_spec("kv-A3fB").unwrap(),
            vec![IdRef::Hash("A3fB".to_string())]
        );
    }

    #[test]
    fn parse_hash_id_mixed_comma() {
        assert_eq!(
            parse_id_spec("1,kv-A3fB,12").unwrap(),
            vec![
                IdRef::Numeric(1),
                IdRef::Hash("A3fB".to_string()),
                IdRef::Numeric(12),
            ]
        );
    }

    #[test]
    fn parse_hash_id_empty_hash() {
        let result = parse_id_spec("kv-");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty hash"));
    }

    // -- parse_where_clauses --

    #[test]
    fn parse_where_clauses_basic() {
        let clauses = vec!["status=active".to_string(), "priority=high".to_string()];
        let parsed = parse_where_clauses(&clauses).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("status".to_string(), "active".to_string()));
        assert_eq!(parsed[1], ("priority".to_string(), "high".to_string()));
    }

    #[test]
    fn parse_where_clauses_value_with_equals() {
        // Value might contain = sign (split on first only)
        let clauses = vec!["query=key=value".to_string()];
        let parsed = parse_where_clauses(&clauses).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ("query".to_string(), "key=value".to_string()));
    }

    #[test]
    fn parse_where_clauses_empty() {
        let clauses: Vec<String> = vec![];
        let parsed = parse_where_clauses(&clauses).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parse_where_clauses_rejects_invalid() {
        let clauses = vec![
            "valid=clause".to_string(),
            "noequalssign".to_string(),
            "also=valid".to_string(),
        ];
        let err = parse_where_clauses(&clauses).unwrap_err();
        assert!(err.contains("noequalssign"));
        assert!(err.contains("expected format key=value"));
    }
}
