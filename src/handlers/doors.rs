//! Handler for `mx doors`. All the IO the pure logic in `crate::doors` refuses
//! to do: stdin, the kv store, the fire log, stdout.
//!
//! The hook path touches the kv file and nothing else. No SurrealDB, no network,
//! no `kn-` resolution — a door prints a pointer and the reader decides whether
//! to follow it.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use anyhow::Result;

use crate::cli::DoorsCommands;
use crate::doors::{self, FIRED_KEY, FiredDoor, HookInput, Selection};
use crate::kv::{DataValue, EntryAttrs, HistoryEntry, KvStore};

/// A `(kv key, entry id)` pair — the tuple that identifies one door. The key is
/// part of it because kv ids are 6-char hashes generated per entry, so the same
/// id can legitimately appear under two different keys.
type DoorId = (String, String);

/// One row of the fire log, flattened out of its `data` blob.
struct FireRow {
    session: String,
    key: String,
    entry: String,
    trigger: String,
    ts: String,
}

fn field(e: &HistoryEntry, name: &str) -> String {
    e.data
        .as_ref()
        .and_then(|d| d.get(name))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Read the whole fire log. A missing key, a wrong type, or a row with no data
/// blob all degrade to "no fires recorded" — the hook must keep working when the
/// log is absent, because that is exactly its state before the first fire.
fn read_fire_log(store: &KvStore) -> Vec<FireRow> {
    let Ok(DataValue::History { entries, .. }) = store.get(FIRED_KEY) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| FireRow {
            session: field(e, "session"),
            key: field(e, "key"),
            entry: field(e, "entry"),
            trigger: field(e, "trigger"),
            ts: e.ts.clone(),
        })
        .collect()
}

/// Split the fire log into this session's dedup set and the all-time fire counts
/// that drive least-fired-first ordering.
fn fire_state(rows: &[FireRow], session: &str) -> (HashSet<DoorId>, HashMap<DoorId, u64>) {
    let mut fired: HashSet<DoorId> = HashSet::new();
    let mut counts: HashMap<DoorId, u64> = HashMap::new();
    for r in rows {
        let pair = (r.key.clone(), r.entry.clone());
        *counts.entry(pair.clone()).or_insert(0) += 1;
        if r.session == session {
            fired.insert(pair);
        }
    }
    (fired, counts)
}

/// Run the matcher for one message. Does not write.
fn evaluate(store: &KvStore, session: &str, prompt: &str, budget: usize) -> Selection {
    let rows = read_fire_log(store);
    let (already, counts) = fire_state(&rows, session);
    let candidates = store.iter_triggered();
    doors::select(prompt, &candidates, &already, &counts, budget)
}

/// Append one fire row per opened door and persist. Overflow (`deferred`) is
/// deliberately NOT recorded, which is what keeps a deferred door eligible on
/// the next prompt.
fn record(store: &mut KvStore, session: &str, fired: &[FiredDoor]) -> Result<()> {
    if fired.is_empty() {
        return Ok(());
    }
    for f in fired {
        store.push(
            FIRED_KEY,
            &f.trigger,
            EntryAttrs {
                data: Some(serde_json::json!({
                    "session": session,
                    "key": f.key,
                    "entry": f.id,
                    "trigger": f.trigger,
                })),
                ..Default::default()
            },
        )?;
    }
    store.save()?;
    Ok(())
}

fn print_fired(fired: &[FiredDoor]) {
    for f in fired {
        println!("{}", f.render());
    }
}

/// `mx doors hook` — the UserPromptSubmit entry point.
///
/// EXIT CODE CONTRACT: this returns `()`, never an error, and the caller always
/// exits 0. On UserPromptSubmit an exit of 2 blocks the turn AND ERASES what the
/// user typed; any other non-zero paints a hook-error notice into every single
/// message. A kv hiccup is never worth either. Failures go to stderr and the
/// prompt proceeds untouched.
fn hook(dry_run: bool, budget: usize) {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        eprintln!("[mx doors] could not read hook input: {e}");
        return;
    }
    let input: HookInput = match serde_json::from_str(&raw) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[mx doors] malformed hook JSON: {e}");
            return;
        }
    };
    if input.prompt.trim().is_empty() {
        return;
    }

    let mut store = match KvStore::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[mx doors] kv unavailable: {e:#}");
            return;
        }
    };

    let selection = evaluate(&store, &input.session_id, &input.prompt, budget);
    if selection.fired.is_empty() {
        return;
    }

    // RECORD BEFORE PRINT. A door that prints without its fire row being
    // persisted has no dedup: it opens again on the next prompt, and the one
    // after, forever. That is the exact shape of the migration hazard — if
    // `doors_fired` is missing from the schema every push fails, and printing
    // first would turn a one-line setup mistake into a door that shouts on
    // every message. Failing to record means failing to fire.
    if !dry_run && let Err(e) = record(&mut store, &input.session_id, &selection.fired) {
        eprintln!("[mx doors] could not record fires, no doors opened: {e:#}");
        return;
    }
    print_fired(&selection.fired);
}

fn check(message: &str, session: &str, dry_run: bool, json: bool, budget: usize) -> Result<i32> {
    let mut store = KvStore::from_env()?;
    let selection = evaluate(&store, session, message, budget);
    // Same ordering as the hook: a door that reports without recording is a
    // door with no dedup. Here the error propagates instead of being swallowed,
    // because `check` is run by a person who wants to see it.
    if !dry_run {
        record(&mut store, session, &selection.fired)?;
    }
    if json {
        println!("{}", serde_json::to_string(&selection)?);
    } else {
        print_fired(&selection.fired);
    }
    Ok(crate::kv::EXIT_OK)
}

fn stats(since: Option<&str>, json: bool) -> Result<i32> {
    let store = KvStore::from_env()?;
    let cutoff = match since {
        Some(s) => Some(crate::kv::parse_relative_time(s).map_err(|e| {
            anyhow::anyhow!(
                "{e} -- --since takes a relative window of minutes or longer (e.g. 30m, 24h, 7d, 2w); seconds are not a unit"
            )
        })?),
        None => None,
    };

    let all_rows = read_fire_log(&store);

    // "Never fired" is computed over the WHOLE log, deliberately. It is the
    // signal used to decide a door is bad and should be pruned, so scoping it to
    // the --since window would report every door that fired only before the
    // window as one that has never fired at all. --since narrows the COUNTS, not
    // the definition of never.
    let ever_fired: HashSet<DoorId> = all_rows
        .iter()
        .map(|r| (r.key.clone(), r.entry.clone()))
        .collect();

    let rows: Vec<&FireRow> = all_rows
        .iter()
        .filter(|r| match cutoff {
            Some(c) => chrono::DateTime::parse_from_rfc3339(&r.ts)
                .map(|t| t.with_timezone(&chrono::Utc) >= c)
                .unwrap_or(false),
            None => true,
        })
        .collect();

    let mut per_entry: HashMap<DoorId, u64> = HashMap::new();
    let mut per_trigger: HashMap<String, u64> = HashMap::new();
    for r in &rows {
        *per_entry
            .entry((r.key.clone(), r.entry.clone()))
            .or_insert(0) += 1;
        *per_trigger.entry(r.trigger.clone()).or_insert(0) += 1;
    }

    let never: Vec<DoorId> = store
        .iter_triggered()
        .iter()
        .map(|c| (c.key.to_string(), c.id.to_string()))
        .filter(|p| !ever_fired.contains(p))
        .collect();

    if json {
        let entries: Vec<serde_json::Value> = per_entry
            .iter()
            .map(|((k, e), n)| serde_json::json!({"key": k, "entry": e, "fires": n}))
            .collect();
        let triggers: Vec<serde_json::Value> = per_trigger
            .iter()
            .map(|(t, n)| serde_json::json!({"trigger": t, "fires": n}))
            .collect();
        let never_json: Vec<serde_json::Value> = never
            .iter()
            .map(|(k, e)| serde_json::json!({"key": k, "entry": e}))
            .collect();
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "total_fires": rows.len(),
                "entries": entries,
                "triggers": triggers,
                "never_fired": never_json,
            }))?
        );
        return Ok(crate::kv::EXIT_OK);
    }

    println!("{} fires recorded", rows.len());
    let mut entries: Vec<_> = per_entry.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for ((k, e), n) in &entries {
        println!("  {:>4}  {}/kv-{}", n, k, e);
    }
    let mut triggers: Vec<_> = per_trigger.into_iter().collect();
    triggers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if !triggers.is_empty() {
        println!("by trigger:");
        for (t, n) in &triggers {
            println!("  {:>4}  {}", n, t);
        }
    }
    if !never.is_empty() {
        println!("never fired:");
        for (k, e) in &never {
            println!("        {}/kv-{}", k, e);
        }
    }
    Ok(crate::kv::EXIT_OK)
}

fn reset(session: Option<&str>) -> Result<i32> {
    let mut store = KvStore::from_env()?;
    let removed = match store.data.entries.get_mut(FIRED_KEY) {
        Some(DataValue::History { entries, .. }) => {
            let before = entries.len();
            match session {
                Some(s) => entries.retain(|e| field(e, "session") != s),
                None => entries.clear(),
            }
            before - entries.len()
        }
        _ => 0,
    };
    if removed > 0 {
        store.save()?;
    }
    eprintln!("[mx doors] cleared {removed} fire rows");
    Ok(crate::kv::EXIT_OK)
}

pub(crate) fn handle_doors(command: DoorsCommands) -> Result<i32> {
    match command {
        DoorsCommands::Hook { dry_run, budget } => {
            hook(dry_run, budget);
            Ok(crate::kv::EXIT_OK)
        }
        DoorsCommands::Check {
            message,
            session,
            dry_run,
            json,
            budget,
        } => check(&message, &session, dry_run, json, budget),
        DoorsCommands::Stats { since, json } => stats(since.as_deref(), json),
        DoorsCommands::Reset { session } => reset(session.as_deref()),
    }
}
