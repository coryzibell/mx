//! Handler for `mx kv` subcommands. Wires CLI to the KV engine.

use std::collections::HashSet;

use anyhow::Result;

use crate::cli::{CreateType, DumpFormat, KvCommands};
use crate::kv::{self, IdRef, KvError, KvStore, resolve_time_range};

/// Map a KvError to the appropriate exit code.
fn exit_code_for(err: &KvError) -> Option<i32> {
    match err {
        KvError::KeyNotFound(_) => Some(kv::EXIT_KEY_NOT_FOUND),
        KvError::TypeMismatch { .. } => Some(kv::EXIT_TYPE_MISMATCH),
        KvError::SchemaMissing(_) => Some(kv::EXIT_SCHEMA_MISSING),
        KvError::EntryNotFound { .. } => Some(kv::EXIT_INVALID_INPUT),
        KvError::AmbiguousId { .. } => Some(kv::EXIT_INVALID_INPUT),
        KvError::DataValidation { .. } => Some(kv::EXIT_INVALID_INPUT),
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
    for (key, _vtype, _desc) in store.keys() {
        if let Ok(Some(mem)) = store.get_memory(key) {
            println!();
            println!("--- {} ---", key);
            print_resolved_memory(mem, verbose);
        }
    }
}

/// Parse a single token into an `IdRef`.
///
/// - Starts with `kv-` -> strip prefix, treat as stable ID (`IdRef::Id`)
/// - Pure digits -> numeric index (`IdRef::Index`)
/// - Otherwise -> error
fn parse_single_id(token: &str) -> Result<IdRef, String> {
    let token = token.trim();
    if let Some(id_str) = token.strip_prefix("kv-") {
        if id_str.is_empty() {
            return Err("empty ID after 'kv-' prefix".to_string());
        }
        Ok(IdRef::Id(id_str.to_string()))
    } else {
        let idx: u64 = token
            .parse()
            .map_err(|_| format!("invalid ID '{}'", token))?;
        Ok(IdRef::Index(idx))
    }
}

/// Parse an ID specification into a list of `IdRef`s.
///
/// Accepted formats:
/// - Single numeric index: "35" -> [Index(35)]
/// - Stable ID: "kv-A3fB" -> [Id("A3fB")]
/// - Numeric range: "35-64" -> [Index(35), ..., Index(64)]
/// - Comma-separated (can mix): "1,kv-A3fB,12" -> [Index(1), Id("A3fB"), Index(12)]
fn parse_id_spec(spec: &str) -> Result<Vec<IdRef>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty ID specification".to_string());
    }

    if spec.contains(',') {
        // Comma-separated list (can mix numeric index and stable ID)
        spec.split(',')
            .map(|s| parse_single_id(s.trim()))
            .collect::<Result<Vec<_>, _>>()
    } else if spec.starts_with("kv-") {
        // Single stable ID
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
        Ok((start..=end).map(IdRef::Index).collect())
    } else {
        // Single numeric index
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

// ---------------------------------------------------------------------------
// Set input parsing
// ---------------------------------------------------------------------------

/// Parsed input for the `kv set` subcommand.
#[derive(Debug, PartialEq)]
enum SetInput {
    /// A single scalar value (string or counter).
    Scalar(String),
    /// A single field=value for a state type (legacy two-arg syntax).
    SingleField { field: String, value: String },
    /// Multiple field=value pairs for batch state set (from positional args or JSON object).
    BatchFields(Vec<(String, String)>),
    /// Parsed JSON array → positional values for tensor set.
    JsonArray(Vec<String>),
    /// No input provided.
    None,
}

/// Parse positional args into a `SetInput`.
///
/// Rules:
/// - 0 args → None
/// - 1 arg without `=` → Scalar
/// - 1 arg with `=` → BatchFields (of 1)
/// - 2 args, neither has `=` → SingleField (legacy backward compat)
/// - 2 args, any has `=` → BatchFields
/// - 3+ args → BatchFields (all must have `=`)
fn parse_positional_args(args: &[String]) -> Result<SetInput, String> {
    match args.len() {
        0 => Ok(SetInput::None),
        1 => {
            let arg = &args[0];
            if arg.contains('=') {
                let (k, v) = arg.split_once('=').unwrap();
                if k.is_empty() {
                    return Err(format!("empty field name in '{}'", arg));
                }
                Ok(SetInput::BatchFields(vec![(k.to_string(), v.to_string())]))
            } else {
                Ok(SetInput::Scalar(arg.clone()))
            }
        }
        2 => {
            let has_eq_0 = args[0].contains('=');
            let has_eq_1 = args[1].contains('=');
            if !has_eq_0 && !has_eq_1 {
                // Legacy: mx kv set <key> <field> <value>
                Ok(SetInput::SingleField {
                    field: args[0].clone(),
                    value: args[1].clone(),
                })
            } else {
                // Both should be key=value pairs
                let mut pairs = Vec::with_capacity(2);
                for arg in args {
                    match arg.split_once('=') {
                        Some((k, v)) => {
                            if k.is_empty() {
                                return Err(format!("empty field name in '{}'", arg));
                            }
                            pairs.push((k.to_string(), v.to_string()));
                        }
                        None => {
                            return Err(format!("expected key=value pair, got '{}'", arg));
                        }
                    }
                }
                Ok(SetInput::BatchFields(pairs))
            }
        }
        _ => {
            // 3+ args: all must be key=value
            let mut pairs = Vec::with_capacity(args.len());
            for arg in args {
                match arg.split_once('=') {
                    Some((k, v)) => {
                        if k.is_empty() {
                            return Err(format!("empty field name in '{}'", arg));
                        }
                        pairs.push((k.to_string(), v.to_string()));
                    }
                    None => {
                        return Err(format!("expected key=value pair, got '{}'", arg));
                    }
                }
            }
            Ok(SetInput::BatchFields(pairs))
        }
    }
}

/// Parse `--json` input into a `SetInput`.
///
/// - If the string is "-", reads from stdin.
/// - Object → extract pairs, route to BatchFields
/// - Array of numbers/strings → extract as strings, route to JsonArray
/// - Other → error
fn parse_json_input(raw: &str) -> Result<SetInput, String> {
    let json_str = if raw == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("failed to read JSON from stdin: {}", e))?;
        buf
    } else {
        raw.to_string()
    };

    let value: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("invalid JSON: {}", e))?;

    match value {
        serde_json::Value::Object(map) => {
            let pairs: Vec<(String, String)> = map
                .into_iter()
                .map(|(k, v)| {
                    let v_str = match v {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (k, v_str)
                })
                .collect();
            if pairs.is_empty() {
                return Err("JSON object is empty".to_string());
            }
            Ok(SetInput::BatchFields(pairs))
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Err("JSON array is empty".to_string());
            }
            let values: Vec<String> = arr
                .into_iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    other => other.to_string(),
                })
                .collect();
            Ok(SetInput::JsonArray(values))
        }
        _ => Err(format!(
            "--json value must be an object or array, got {}",
            match value {
                serde_json::Value::String(_) => "string",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Null => "null",
                _ => "unknown",
            }
        )),
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
        KvCommands::Get {
            key,
            id,
            memory,
            json,
        } => {
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
                        if json {
                            println!("{}", serde_json::to_string_pretty(&hits)?);
                            return Ok(kv::EXIT_OK);
                        }

                        for hit in &hits {
                            println!(
                                "{}",
                                kv::format_entry_line(
                                    hit.index, &hit.id, &hit.value, &hit.ts, &hit.data
                                )
                            );
                        }

                        // Report missing IDs
                        let found_indexes: HashSet<u64> = hits.iter().map(|h| h.index).collect();
                        let found_ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
                        let missing: Vec<String> = ids
                            .iter()
                            .filter(|id_ref| match id_ref {
                                IdRef::Index(n) => !found_indexes.contains(n),
                                IdRef::Id(h) => {
                                    !found_ids.iter().any(|fid| fid.starts_with(h.as_str()))
                                }
                            })
                            .map(|id_ref| match id_ref {
                                IdRef::Index(n) => n.to_string(),
                                IdRef::Id(h) => format!("kv-{}", h),
                            })
                            .collect();
                        if !missing.is_empty() {
                            eprintln!("note: IDs not found: {}", missing.join(", "));
                        }

                        if memory {
                            // Resolve per-entry memory pointers
                            for hit in &hits {
                                if let Some(ref mem) = hit.memory {
                                    print_resolved_memory(mem, verbose);
                                } else if hit.value.starts_with("kn-") {
                                    // Legacy: resolve entry value as kn- reference
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
                        if json {
                            match val {
                                kv::DataValue::History { entries, .. } => {
                                    let hits: Vec<kv::SearchHit> = entries
                                        .iter()
                                        .map(|e| kv::SearchHit {
                                            index: e.index,
                                            id: e.id.clone(),
                                            value: e.value.clone(),
                                            ts: e.ts.clone(),
                                            data: e.data.clone(),
                                            memory: e.memory.clone(),
                                        })
                                        .collect();
                                    println!("{}", serde_json::to_string_pretty(&hits)?);
                                }
                                kv::DataValue::List { items, .. } => {
                                    let hits: Vec<kv::SearchHit> = items
                                        .iter()
                                        .map(|e| kv::SearchHit {
                                            index: e.index,
                                            id: e.id.clone(),
                                            value: e.value.clone(),
                                            ts: e.ts.clone(),
                                            data: e.data.clone(),
                                            memory: e.memory.clone(),
                                        })
                                        .collect();
                                    println!("{}", serde_json::to_string_pretty(&hits)?);
                                }
                                _ => {
                                    let json_val =
                                        serde_json::json!({"value": kv::format_value(val)});
                                    println!("{}", serde_json::to_string_pretty(&json_val)?);
                                }
                            }
                            return Ok(kv::EXIT_OK);
                        }
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
            args,
            json,
            memory,
            id,
        } => {
            // Per-entry memory: --id + --memory targets a specific entry
            // (clap enforces that --id requires --memory at parse time)
            if let Some(ref id_str) = id {
                let id_ref = match parse_single_id(id_str) {
                    Ok(r) => r,
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                };
                match store.set_entry_memory(&key, &id_ref, memory) {
                    Ok(()) => {
                        store.save()?;
                        return Ok(kv::EXIT_OK);
                    }
                    Err(e) => return handle_kv_err(e),
                }
            }

            // Reject --json + positional args together
            if json.is_some() && !args.is_empty() {
                eprintln!("Error: --json and positional arguments cannot be combined");
                return Ok(kv::EXIT_INVALID_INPUT);
            }

            // Parse input into a SetInput
            let input = if let Some(json_str) = json {
                match parse_json_input(&json_str) {
                    Ok(i) => i,
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                }
            } else {
                match parse_positional_args(&args) {
                    Ok(i) => i,
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                }
            };

            let mut did_something = false;

            // Dispatch based on parsed input
            match input {
                SetInput::None => {}
                SetInput::Scalar(val) => {
                    let result = store.set(&key, &val, None);
                    match result {
                        Ok(()) => {
                            did_something = true;
                        }
                        Err(e) => {
                            if memory.is_none() {
                                return handle_kv_err(e);
                            }
                            match &e {
                                KvError::TypeMismatch { .. } => {
                                    eprintln!(
                                        "Warning: value not set (type does not support set); memory pointer updated"
                                    );
                                }
                                _ => return handle_kv_err(e),
                            }
                        }
                    }
                }
                SetInput::SingleField { field, value } => {
                    let result = store.set(&key, &value, Some(&field));
                    match result {
                        Ok(()) => {
                            did_something = true;
                        }
                        Err(e) => {
                            if memory.is_none() {
                                return handle_kv_err(e);
                            }
                            match &e {
                                KvError::TypeMismatch { .. } => {
                                    eprintln!(
                                        "Warning: value not set (type does not support set); memory pointer updated"
                                    );
                                }
                                _ => return handle_kv_err(e),
                            }
                        }
                    }
                }
                SetInput::BatchFields(pairs) => match store.set_state_batch(&key, &pairs) {
                    Ok(()) => {
                        did_something = true;
                    }
                    Err(e) => {
                        if memory.is_none() {
                            return handle_kv_err(e);
                        }
                        match &e {
                            KvError::TypeMismatch { .. } => {
                                eprintln!(
                                    "Warning: value not set (type does not support batch set); memory pointer updated"
                                );
                            }
                            _ => return handle_kv_err(e),
                        }
                    }
                },
                SetInput::JsonArray(values) => match store.set_tensor_batch(&key, &values) {
                    Ok(()) => {
                        did_something = true;
                    }
                    Err(e) => {
                        if memory.is_none() {
                            return handle_kv_err(e);
                        }
                        match &e {
                            KvError::TypeMismatch { .. } => {
                                eprintln!(
                                    "Warning: value not set (type does not support tensor set); memory pointer updated"
                                );
                            }
                            _ => return handle_kv_err(e),
                        }
                    }
                },
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

        KvCommands::Push {
            key,
            value,
            data,
            memory,
            create,
            max_entries,
        } => {
            // Handle --create: auto-add key to schema if missing
            if let Some(ref create_type) = create {
                let type_str = match create_type {
                    CreateType::History => "history",
                    CreateType::List => "list",
                };
                if !store.schema.keys.contains_key(&key)
                    && let Err(e) = store.add_key_to_schema(&key, type_str, max_entries)
                {
                    return handle_kv_err(e);
                }
                // If key already exists, silently ignore --create
            }

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

            match store.push(&key, &value, parsed_data, memory) {
                Ok(result) => {
                    store.save()?;
                    println!("kv-{} ({})", result.id, result.index);
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
                        entry.index,
                        &entry.id,
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
            json,
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
                Ok(hits) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&hits)?);
                        return Ok(kv::EXIT_OK);
                    }
                    for hit in &hits {
                        println!(
                            "{}",
                            kv::format_entry_line(
                                hit.index, &hit.id, &hit.value, &hit.ts, &hit.data
                            )
                        );
                        if memory {
                            if let Some(ref mem) = hit.memory {
                                print_resolved_memory(mem, verbose);
                            } else if hit.value.starts_with("kn-") {
                                print_resolved_memory(&hit.value, verbose);
                            }
                        }
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
            json,
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
                Ok(hits) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&hits)?);
                        return Ok(kv::EXIT_OK);
                    }
                    for hit in &hits {
                        println!(
                            "{}",
                            kv::format_entry_line(
                                hit.index, &hit.id, &hit.value, &hit.ts, &hit.data
                            )
                        );
                        if memory {
                            if let Some(ref mem) = hit.memory {
                                print_resolved_memory(mem, verbose);
                            } else if hit.value.starts_with("kn-") {
                                print_resolved_memory(&hit.value, verbose);
                            }
                        }
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
            json,
        } => match store.since(&key, &timeref) {
            Ok(entries) => {
                if json {
                    let hits: Vec<kv::SearchHit> = entries
                        .iter()
                        .map(|e| kv::SearchHit {
                            index: e.index,
                            id: e.id.clone(),
                            value: e.value.clone(),
                            ts: e.ts.clone(),
                            data: e.data.clone(),
                            memory: e.memory.clone(),
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&hits)?);
                    return Ok(kv::EXIT_OK);
                }
                for entry in &entries {
                    println!(
                        "{}",
                        kv::format_entry_line(
                            entry.index,
                            &entry.id,
                            &entry.value,
                            &entry.ts,
                            &entry.data
                        )
                    );
                    if memory {
                        if let Some(ref mem) = entry.memory {
                            print_resolved_memory(mem, verbose);
                        } else if entry.value.starts_with("kn-") {
                            print_resolved_memory(&entry.value, verbose);
                        }
                    }
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
            // Parse --id as an IdRef (numeric index or stable ID)
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

        KvCommands::Update {
            key,
            value,
            id,
            data,
        } => {
            // 1. Reject the no-op case (both fields missing).
            if value.is_none() && data.is_none() {
                eprintln!("Error: provide a value argument and/or --data to update");
                return Ok(kv::EXIT_INVALID_INPUT);
            }

            // 2. Parse the ID into IdRef (numeric or kv-HASH).
            let id_ref = match parse_single_id(&id) {
                Ok(r) => r,
                Err(msg) => {
                    eprintln!("Error: {}", msg);
                    return Ok(kv::EXIT_INVALID_INPUT);
                }
            };

            // 3. Parse --data as JSON object (mirror push's checks).
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
                    if val.as_object().is_some_and(|o| o.is_empty()) && value.is_none() {
                        eprintln!(
                            "Error: --data is an empty object and no value was given — nothing to update"
                        );
                        return Ok(kv::EXIT_INVALID_INPUT);
                    }
                    Some(val)
                }
                None => None,
            };

            // 4. Dispatch to engine.
            match store.update_entry(&key, &id_ref, value.as_deref(), parsed_data) {
                Ok(result) => {
                    store.save()?;
                    println!("Updated entry {} (kv-{})", result.index, result.id);
                    Ok(kv::EXIT_OK)
                }
                Err(e) => handle_kv_err(e),
            }
        }

        KvCommands::Search {
            key,
            query,
            memory,
            json,
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
                    if json {
                        println!("{}", serde_json::to_string_pretty(&hits)?);
                        return Ok(kv::EXIT_OK);
                    }
                    if hits.is_empty() {
                        eprintln!("No matching entries");
                        Ok(kv::EXIT_OK)
                    } else {
                        for hit in &hits {
                            println!(
                                "{}",
                                kv::format_entry_line(
                                    hit.index, &hit.id, &hit.value, &hit.ts, &hit.data
                                )
                            );
                            if memory {
                                if let Some(ref mem) = hit.memory {
                                    print_resolved_memory(mem, verbose);
                                } else if hit.value.starts_with("kn-") {
                                    print_resolved_memory(&hit.value, verbose);
                                }
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
            json,
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
                    if json {
                        let mut json_val = serde_json::json!({"count": result.matched});
                        if let Some(total) = result.total {
                            json_val["total"] = serde_json::json!(total);
                        }
                        if let Some(ref ts) = result.latest_ts {
                            json_val["latest_ts"] = serde_json::json!(ts);
                        }
                        println!("{}", serde_json::to_string_pretty(&json_val)?);
                        return Ok(kv::EXIT_OK);
                    }
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

        KvCommands::Migrate {
            key,
            prune,
            dry_run,
        } => match store.migrate(&key, prune, dry_run) {
            Ok(result) => {
                let verb = if dry_run { "would modify" } else { "modified" };
                println!(
                    "Examined {} entries, {} {}",
                    result.examined, verb, result.modified
                );

                for change in &result.changes {
                    let mut parts = Vec::new();
                    if !change.fields_added.is_empty() {
                        parts.push(format!("added: {}", change.fields_added.join(", ")));
                    }
                    if !change.fields_pruned.is_empty() {
                        parts.push(format!("pruned: {}", change.fields_pruned.join(", ")));
                    }
                    println!(
                        "  kv-{} ({}): {}",
                        change.id,
                        change.index,
                        parts.join("; ")
                    );
                }

                for warning in &result.warnings {
                    eprintln!("warning: {}", warning);
                }

                if dry_run && result.modified > 0 {
                    eprintln!("(dry run -- no changes written)");
                }

                if !dry_run && result.modified > 0 {
                    store.save()?;
                }

                Ok(kv::EXIT_OK)
            }
            Err(e) => handle_kv_err(e),
        },

        KvCommands::Keys => {
            let keys = store.keys();
            for (name, vtype, desc) in &keys {
                match desc {
                    Some(d) => println!("{:30} {:10} {}", name, vtype, d),
                    None => println!("{:30} {:10}", name, vtype),
                }
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
        assert_eq!(parse_id_spec("35").unwrap(), vec![IdRef::Index(35)]);
    }

    #[test]
    fn parse_single_id_zero() {
        assert_eq!(parse_id_spec("0").unwrap(), vec![IdRef::Index(0)]);
    }

    #[test]
    fn parse_range() {
        assert_eq!(
            parse_id_spec("3-7").unwrap(),
            vec![
                IdRef::Index(3),
                IdRef::Index(4),
                IdRef::Index(5),
                IdRef::Index(6),
                IdRef::Index(7),
            ]
        );
    }

    #[test]
    fn parse_range_single_element() {
        assert_eq!(parse_id_spec("5-5").unwrap(), vec![IdRef::Index(5)]);
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
            vec![IdRef::Index(1), IdRef::Index(5), IdRef::Index(12)]
        );
    }

    #[test]
    fn parse_comma_separated_with_spaces() {
        assert_eq!(
            parse_id_spec("1, 5, 12").unwrap(),
            vec![IdRef::Index(1), IdRef::Index(5), IdRef::Index(12)]
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

    // -- ID parsing --

    #[test]
    fn parse_id_single() {
        assert_eq!(
            parse_id_spec("kv-A3fB").unwrap(),
            vec![IdRef::Id("A3fB".to_string())]
        );
    }

    #[test]
    fn parse_id_mixed_comma() {
        assert_eq!(
            parse_id_spec("1,kv-A3fB,12").unwrap(),
            vec![
                IdRef::Index(1),
                IdRef::Id("A3fB".to_string()),
                IdRef::Index(12),
            ]
        );
    }

    #[test]
    fn parse_id_empty() {
        let result = parse_id_spec("kv-");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty ID"));
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

    // -- parse_positional_args --

    #[test]
    fn positional_empty_args() {
        assert_eq!(parse_positional_args(&[]).unwrap(), SetInput::None,);
    }

    #[test]
    fn positional_single_scalar() {
        assert_eq!(
            parse_positional_args(&["hello".to_string()]).unwrap(),
            SetInput::Scalar("hello".to_string()),
        );
    }

    #[test]
    fn positional_single_key_value() {
        assert_eq!(
            parse_positional_args(&["goal=finish docs".to_string()]).unwrap(),
            SetInput::BatchFields(vec![("goal".to_string(), "finish docs".to_string())]),
        );
    }

    #[test]
    fn positional_legacy_two_bare_args() {
        assert_eq!(
            parse_positional_args(&["phase".to_string(), "writing".to_string()]).unwrap(),
            SetInput::SingleField {
                field: "phase".to_string(),
                value: "writing".to_string(),
            },
        );
    }

    #[test]
    fn positional_two_key_value_args() {
        assert_eq!(
            parse_positional_args(&["goal=finish docs".to_string(), "phase=writing".to_string(),])
                .unwrap(),
            SetInput::BatchFields(vec![
                ("goal".to_string(), "finish docs".to_string()),
                ("phase".to_string(), "writing".to_string()),
            ]),
        );
    }

    #[test]
    fn positional_three_plus_key_value_args() {
        assert_eq!(
            parse_positional_args(&[
                "goal=finish docs".to_string(),
                "phase=writing".to_string(),
                "blocker=none".to_string(),
            ])
            .unwrap(),
            SetInput::BatchFields(vec![
                ("goal".to_string(), "finish docs".to_string()),
                ("phase".to_string(), "writing".to_string()),
                ("blocker".to_string(), "none".to_string()),
            ]),
        );
    }

    #[test]
    fn positional_value_containing_equals_splits_first_only() {
        assert_eq!(
            parse_positional_args(&["query=key=value".to_string()]).unwrap(),
            SetInput::BatchFields(vec![("query".to_string(), "key=value".to_string())]),
        );
    }

    #[test]
    fn positional_mixed_two_args_one_has_eq() {
        let result = parse_positional_args(&["goal=finish".to_string(), "writing".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected key=value pair"));
    }

    #[test]
    fn positional_three_args_one_missing_eq() {
        let result = parse_positional_args(&[
            "goal=finish".to_string(),
            "writing".to_string(),
            "blocker=none".to_string(),
        ]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected key=value pair"));
    }

    // -- parse_json_input --

    #[test]
    fn json_object_parsing() {
        let input = parse_json_input(r#"{"goal":"finish docs","phase":"writing"}"#).unwrap();
        match input {
            SetInput::BatchFields(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert!(pairs.contains(&("goal".to_string(), "finish docs".to_string())));
                assert!(pairs.contains(&("phase".to_string(), "writing".to_string())));
            }
            _ => panic!("expected BatchFields"),
        }
    }

    #[test]
    fn json_array_parsing() {
        assert_eq!(
            parse_json_input("[0.4, 0.6, 0.5]").unwrap(),
            SetInput::JsonArray(vec![
                "0.4".to_string(),
                "0.6".to_string(),
                "0.5".to_string(),
            ]),
        );
    }

    #[test]
    fn json_invalid_input() {
        let result = parse_json_input("not json at all");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn json_string_rejected() {
        let result = parse_json_input(r#""just a string""#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be an object or array"));
    }

    #[test]
    fn json_empty_object_rejected() {
        let result = parse_json_input("{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn json_empty_array_rejected() {
        let result = parse_json_input("[]");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn json_object_with_numeric_value() {
        let input = parse_json_input(r#"{"temperature":0.75}"#).unwrap();
        match input {
            SetInput::BatchFields(pairs) => {
                assert_eq!(pairs, vec![("temperature".to_string(), "0.75".to_string())]);
            }
            _ => panic!("expected BatchFields"),
        }
    }

    // -- S3: --json + positional mutual exclusion --

    #[test]
    fn json_and_positional_args_mutual_exclusion() {
        // Simulate the handler guard: --json provided AND positional args non-empty
        let json: Option<String> = Some(r#"{"goal":"test"}"#.to_string());
        let args: Vec<String> = vec!["goal=test".to_string()];
        assert!(
            json.is_some() && !args.is_empty(),
            "guard should reject --json combined with positional args"
        );
    }

    // -- S4: JSON coercion behavior --

    #[test]
    fn json_object_with_boolean_value() {
        let input = parse_json_input(r#"{"flag":true}"#).unwrap();
        match input {
            SetInput::BatchFields(pairs) => {
                assert_eq!(pairs, vec![("flag".to_string(), "true".to_string())]);
            }
            _ => panic!("expected BatchFields"),
        }
    }

    #[test]
    fn json_object_with_null_value() {
        let input = parse_json_input(r#"{"val":null}"#).unwrap();
        match input {
            SetInput::BatchFields(pairs) => {
                assert_eq!(pairs, vec![("val".to_string(), "null".to_string())]);
            }
            _ => panic!("expected BatchFields"),
        }
    }

    #[test]
    fn json_object_with_integer_value() {
        let input = parse_json_input(r#"{"count":42}"#).unwrap();
        match input {
            SetInput::BatchFields(pairs) => {
                assert_eq!(pairs, vec![("count".to_string(), "42".to_string())]);
            }
            _ => panic!("expected BatchFields"),
        }
    }

    // -- W2: empty field name rejection --

    #[test]
    fn positional_empty_field_name_rejected() {
        let result = parse_positional_args(&["=hello".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty field name"));
    }

    #[test]
    fn positional_empty_field_name_in_batch_rejected() {
        let result = parse_positional_args(&["goal=finish".to_string(), "=oops".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty field name"));
    }

    #[test]
    fn positional_empty_field_name_in_three_plus_rejected() {
        let result = parse_positional_args(&[
            "goal=finish".to_string(),
            "phase=writing".to_string(),
            "=bad".to_string(),
        ]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty field name"));
    }
}
