//! Handler for `mx doors`. All the IO the pure logic in `crate::doors` refuses
//! to do: stdin, the kv store, the fire log, stdout.
//!
//! The hook path touches the kv file and nothing else. No SurrealDB, no network,
//! no `kn-` resolution — a door prints a pointer and the reader decides whether
//! to follow it.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::cli::DoorsCommands;
use crate::doors::{
    self, Explanation, FIRED_KEY, FiredDoor, Hint, HookInput, Mode, NamesUs, SESSION_KEY,
    Selection, SessionRow, SessionState,
};
use crate::kv::{DataFieldType, DataValue, EntryAttrs, HistoryEntry, KvStore, TriggeredRef};

/// What `mx doors mode` prints when the schema has no `doors_session` key.
/// The CHANGELOG carries the same text.
const SESSION_SCHEMA_TOML: &str = r#"[keys.doors_session]
type = "history"
max_entries = 1000
description = "Doors session state: mode changes, hints shown, and foreign pastes read in book mode."

[keys.doors_session.data.session]
type = "string"
required = true

[keys.doors_session.data.event]
type = "string"
required = true

[keys.doors_session.data.mode]
type = "string"
required = false

[keys.doors_session.data.by]
type = "string"
required = false

[keys.doors_session.data.kind]
type = "string"
required = false

[keys.doors_fired.data.place]
type = "string"
required = false
"#;

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

/// Whether session state can be read and written: the schema declares
/// `doors_session` and there is a session to key it by. Without it the hook
/// reads house mode, writes nothing and hints nothing.
fn tracks_sessions(store: &KvStore, session: &str) -> bool {
    !session.is_empty() && store.schema.keys.contains_key(SESSION_KEY)
}

/// This session's `doors_session` rows, newest first.
fn read_session(store: &KvStore, session: &str) -> Vec<SessionRow> {
    let Ok(DataValue::History { entries, .. }) = store.get(SESSION_KEY) else {
        return Vec::new();
    };
    let opt = |e: &HistoryEntry, n: &str| Some(field(e, n)).filter(|v| !v.is_empty());
    entries
        .iter()
        .filter(|e| field(e, "session") == session)
        .map(|e| SessionRow {
            event: field(e, "event"),
            mode: opt(e, "mode"),
            kind: opt(e, "kind"),
            by: opt(e, "by"),
            ts: e.ts.clone(),
        })
        .collect()
}

fn session_state(store: &KvStore, session: &str) -> SessionState {
    if tracks_sessions(store, session) {
        doors::session_state(&read_session(store, session))
    } else {
        SessionState::default()
    }
}

fn push_session(store: &mut KvStore, event: &str, mut data: serde_json::Value) -> Result<()> {
    data["event"] = event.into();
    store.push(
        SESSION_KEY,
        event,
        EntryAttrs {
            data: Some(data),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// The per-pool budget: `--budget`, else `MX_DOORS_BUDGET` when it parses
/// (0 is the kill switch), else the default. A bad env value is ignored rather
/// than reported, because the hook must never fail on it.
fn resolve_budget(flag: Option<usize>) -> usize {
    flag.or_else(|| std::env::var("MX_DOORS_BUDGET").ok()?.trim().parse().ok())
        .unwrap_or(doors::DEFAULT_BUDGET)
}

/// The words that mark text as ours, from `MX_DOORS_NAMES` (comma-separated).
/// Unset or empty leaves only the standalone-`Q` rule.
fn names_us() -> NamesUs {
    NamesUs::new(
        std::env::var("MX_DOORS_NAMES")
            .unwrap_or_default()
            .split(',')
            .map(str::to_string),
    )
}

type Matcher<T> = fn(
    &str,
    &Mode,
    &NamesUs,
    &[TriggeredRef<'_>],
    &HashSet<DoorId>,
    &HashMap<DoorId, u64>,
    usize,
) -> T;

/// Run `matcher` (`doors::select`, `doors::explain` or `doors::route`) for one
/// message. Does not write.
fn evaluate<T>(
    store: &KvStore,
    session: &str,
    prompt: &str,
    mode: &Mode,
    budget: usize,
    matcher: Matcher<T>,
) -> T {
    let rows = read_fire_log(store);
    let (already, counts) = fire_state(&rows, session);
    let candidates = store.iter_triggered();
    matcher(
        prompt,
        mode,
        &names_us(),
        &candidates,
        &already,
        &counts,
        budget,
    )
}

/// Whether the store's schema accepts a string `name` field on fire rows.
/// Schemas written before a field existed reject undeclared fields, and a
/// rejected push means no door fires at all, so a field is only written where
/// it is declared as a string.
fn schema_takes(store: &KvStore, name: &str) -> bool {
    store.schema.keys.get(FIRED_KEY).is_some_and(|k| {
        k.data.as_ref().is_none_or(|d| {
            d.get(name)
                .is_some_and(|f| f.field_type == DataFieldType::String)
        })
    })
}

/// Append one fire row per opened door, without saving. Overflow (`deferred`)
/// is deliberately NOT recorded, which is what keeps a deferred door eligible
/// on the next prompt.
fn push_fires(store: &mut KvStore, session: &str, fired: &[FiredDoor]) -> Result<()> {
    let with_origin = schema_takes(store, "origin");
    let with_place = schema_takes(store, "place");
    for f in fired {
        let mut data = serde_json::json!({
            "session": session,
            "key": f.key,
            "entry": f.id,
            "trigger": f.trigger,
        });
        if with_origin {
            data["origin"] = f.origin.into();
        }
        if with_place {
            data["place"] = f.place.as_str().into();
        }
        store.push(
            FIRED_KEY,
            &f.trigger,
            EntryAttrs {
                data: Some(data),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

fn record(store: &mut KvStore, session: &str, fired: &[FiredDoor]) -> Result<()> {
    if fired.is_empty() {
        return Ok(());
    }
    push_fires(store, session, fired)?;
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
///
/// One store load and at most one save per prompt: fire rows, hint rows and the
/// `foreign_paste` row all land in the same write.
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

    let sid = input.session_id.as_str();
    let tracked = tracks_sessions(&store, sid);
    let state = session_state(&store, sid);
    let ex = evaluate(
        &store,
        sid,
        &input.prompt,
        &state.mode,
        budget,
        doors::route,
    );
    let fired = ex.selection().fired;
    let now = Utc::now();
    let (hints, mark_foreign) = if tracked {
        (
            doors::hints(&ex.segments, &state.mode, &state, now),
            doors::foreign_paste(&ex.segments, &state, now),
        )
    } else {
        (Vec::new(), false)
    };
    if fired.is_empty() && hints.is_empty() && !mark_foreign {
        return;
    }
    if dry_run {
        print_fired(&fired);
        print_hints(&hints);
        return;
    }

    // RECORD BEFORE PRINT. A door that prints without its fire row being
    // persisted has no dedup: it opens again on the next prompt, and the one
    // after, forever. That is the exact shape of the migration hazard — if
    // `doors_fired` is missing from the schema every push fails, and printing
    // first would turn a one-line setup mistake into a door that shouts on
    // every message. Failing to record means failing to fire. Hints follow the
    // same rule: one that cannot be recorded would print on every prompt.
    let before = store.data.entries.get(FIRED_KEY).cloned();
    let doors_ok = match push_fires(&mut store, sid, &fired) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[mx doors] could not record fires, no doors opened: {e:#}");
            match before {
                Some(v) => store.data.entries.insert(FIRED_KEY.to_string(), v),
                None => store.data.entries.remove(FIRED_KEY),
            };
            false
        }
    };
    let mut shown: Vec<Hint> = Vec::new();
    for h in hints {
        let row = serde_json::json!({
            "session": sid,
            "kind": h.kind.label(),
            "mode": state.mode.to_string(),
        });
        match push_session(&mut store, "hint", row) {
            Ok(()) => shown.push(h),
            Err(e) => eprintln!("[mx doors] could not record hint, not shown: {e:#}"),
        }
    }
    let marked = mark_foreign
        && push_session(
            &mut store,
            "foreign_paste",
            serde_json::json!({"session": sid}),
        )
        .is_ok();
    if ((doors_ok && !fired.is_empty()) || !shown.is_empty() || marked)
        && let Err(e) = store.save()
    {
        eprintln!("[mx doors] could not save, nothing opened: {e:#}");
        return;
    }
    if doors_ok {
        print_fired(&fired);
    }
    print_hints(&shown);
}

fn print_hints(hints: &[Hint]) {
    for h in hints {
        println!("{}", h.text);
    }
}

/// The id Claude Code puts on a session's paste tags: the first 4 hex of
/// sha256(session id).
fn paste_id(session: &str) -> String {
    Sha256::digest(session.as_bytes())[..2]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The prompt the hook would see: the message (in a channel block when it is
/// `--from` someone), then each paste in Claude Code's envelope.
fn compose(message: &str, session: &str, pastes: &[String], from: Option<&str>) -> String {
    let mut prompt = match from {
        Some(user) => format!("<channel source=\"cli\" user_id=\"{user}\">\n{message}\n</channel>"),
        None => message.to_string(),
    };
    let id = paste_id(session);
    for body in pastes {
        let body = body.strip_suffix('\n').unwrap_or(body);
        prompt.push_str(&format!(
            "\n\n<pasted_content id=\"{id}\">\n{body}\n</pasted_content id=\"{id}\">"
        ));
    }
    prompt
}

const NOT_CLASSIFIED: &str = "typed/channel are never held";

fn print_explanation(ex: &Explanation, source: &str, hints: &[Hint]) {
    println!("mode {} ({source})", ex.mode);
    for (n, seg) in ex.segments.iter().enumerate() {
        let origin = match &seg.origin {
            doors::Origin::Channel { user_id } if !user_id.is_empty() => {
                format!("channel({user_id})")
            }
            o => o.label().to_string(),
        };
        let verdict = match &seg.classified {
            Some(c) => {
                let v = if c.gated {
                    "Unsure(gate)".to_string()
                } else {
                    format!("{:?}", c.verdict)
                };
                let signals: Vec<String> = c.signals.iter().map(|s| s.label()).collect();
                format!("verdict {v} F={} O={} [{}]", c.f, c.o, signals.join(", "))
            }
            None => format!("not classified ({NOT_CLASSIFIED})"),
        };
        println!(
            "segment {}  {}  {} ch  {}  places {}",
            n + 1,
            origin,
            seg.text.chars().count(),
            verdict,
            seg.places
        );
    }
    for d in &ex.doors {
        let place = if d.door.place.is_empty() {
            "-"
        } else {
            &d.door.place
        };
        let pool = if d.status == doors::DoorStatus::Held {
            "-"
        } else {
            d.pool.label()
        };
        println!(
            "{}  {}/kv-{}  {}  seg {}  place {}  pool {}",
            d.door.trigger,
            d.door.key,
            d.door.id,
            d.status.label(),
            d.segment,
            place,
            pool
        );
    }
    for h in hints {
        println!("hint: {}", h.kind.label());
    }
}

fn explanation_json(ex: &Explanation, source: &str, hints: &[Hint]) -> serde_json::Value {
    let segments: Vec<serde_json::Value> = ex
        .segments
        .iter()
        .enumerate()
        .map(|(n, seg)| {
            let user_id = match &seg.origin {
                doors::Origin::Channel { user_id } => Some(user_id.as_str()),
                _ => None,
            };
            let c = seg.classified.as_ref();
            serde_json::json!({
                "segment": n + 1,
                "origin": seg.origin.label(),
                "user_id": user_id,
                "chars": seg.text.chars().count(),
                "verdict": c.map(|c| c.verdict),
                "reason": c.is_none().then_some(NOT_CLASSIFIED),
                "gated": c.map(|c| c.gated),
                "f": c.map(|c| c.f),
                "o": c.map(|c| c.o),
                "signals": c.map(|c| &c.signals),
                "places": seg.places.labels(),
                "quarantined": seg.places.is_empty(),
            })
        })
        .collect();
    let doors: Vec<serde_json::Value> = ex
        .doors
        .iter()
        .map(|d| {
            serde_json::json!({
                "trigger": d.door.trigger,
                "key": d.door.key,
                "id": d.door.id,
                "dig": d.door.dig,
                "origin": d.door.origin,
                "status": d.status,
                "segment": d.segment,
                "place": d.door.place,
                "pool": d.pool,
            })
        })
        .collect();
    let hints: Vec<serde_json::Value> = hints
        .iter()
        .map(|h| serde_json::json!({"kind": h.kind, "text": h.text}))
        .collect();
    let sel = ex.selection();
    serde_json::json!({
        "mode": ex.mode.to_string(),
        "mode_source": source,
        "segments": segments,
        "doors": doors,
        "fired": sel.fired,
        "deferred": sel.deferred,
        "hints": hints,
    })
}

struct CheckArgs {
    message: String,
    session: String,
    dry_run: bool,
    json: bool,
    budget: usize,
    mode: Option<Mode>,
    pastes: Vec<PathBuf>,
    from: Option<String>,
    explain: bool,
}

fn check(a: CheckArgs) -> Result<i32> {
    let pastes = a
        .pastes
        .iter()
        .map(|p| {
            std::fs::read_to_string(p).with_context(|| format!("reading --paste {}", p.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let prompt = compose(&a.message, &a.session, &pastes, a.from.as_deref());
    let mut store = KvStore::from_env()?;
    let state = session_state(&store, &a.session);
    let (mode, source) = match a.mode {
        Some(m) => (m, "--mode"),
        None if state.since.is_some() => (state.mode.clone(), "session"),
        None => (Mode::House, "default"),
    };
    // Explaining never burns dedup, and never records a hint.
    if a.explain {
        let ex = evaluate(&store, &a.session, &prompt, &mode, a.budget, doors::explain);
        let hints = if tracks_sessions(&store, &a.session) {
            doors::hints(&ex.segments, &mode, &state, Utc::now())
        } else {
            Vec::new()
        };
        if a.json {
            println!(
                "{}",
                serde_json::to_string(&explanation_json(&ex, source, &hints))?
            );
        } else {
            print_explanation(&ex, source, &hints);
        }
        return Ok(crate::kv::EXIT_OK);
    }
    let selection: Selection =
        evaluate(&store, &a.session, &prompt, &mode, a.budget, doors::select);
    // Same ordering as the hook: a door that reports without recording is a
    // door with no dedup. Here the error propagates instead of being swallowed,
    // because `check` is run by a person who wants to see it.
    if !a.dry_run {
        record(&mut store, &a.session, &selection.fired)?;
    }
    if a.json {
        println!("{}", serde_json::to_string(&selection)?);
    } else {
        print_fired(&selection.fired);
    }
    Ok(crate::kv::EXIT_OK)
}

/// `mx doors mode [MODE] [--session ID]`: print the session's mode, or write a
/// mode row. Writing the current mode again is deliberate: it opens a fresh
/// epoch, which re-arms the hints and resets the stale timer.
fn mode(new: Option<String>, session: Option<String>) -> Result<i32> {
    let new = match new.as_deref().map(|m| (m, Mode::parse(m))) {
        None => None,
        Some((_, Ok(m))) => Some(m),
        Some((raw, Err(why))) => {
            eprintln!("Error: {raw:?} {why}");
            return Ok(2);
        }
    };
    let session = session
        .or_else(|| std::env::var("CLAUDE_CODE_SESSION_ID").ok())
        .filter(|s| !s.is_empty());
    let Some(session) = session else {
        eprintln!("Error: no session: pass --session ID or set CLAUDE_CODE_SESSION_ID");
        return Ok(1);
    };
    let mut store = KvStore::from_env()?;
    let Some(new) = new else {
        let st = session_state(&store, &session);
        match st.since {
            Some((ts, by)) => println!("{}, since {ts}, set by {by}", st.mode),
            None => println!("house, never set"),
        }
        return Ok(crate::kv::EXIT_OK);
    };
    if !store.schema.keys.contains_key(SESSION_KEY) {
        eprintln!(
            "Error: kv key '{SESSION_KEY}' is not in the schema, so the mode cannot be stored. Add this to the schema:\n\n{SESSION_SCHEMA_TOML}"
        );
        return Ok(1);
    }
    let by = std::env::var("MX_CURRENT_AGENT")
        .ok()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "cli".to_string());
    push_session(
        &mut store,
        "mode",
        serde_json::json!({"session": session, "mode": new.to_string(), "by": by}),
    )?;
    store.save()?;
    println!("{new}");
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
            hook(dry_run, resolve_budget(budget));
            Ok(crate::kv::EXIT_OK)
        }
        DoorsCommands::Check {
            message,
            session,
            dry_run,
            json,
            budget,
            mode,
            pastes,
            from,
            explain,
        } => {
            let mode = match mode.as_deref().map(|m| (m, Mode::parse(m))) {
                None => None,
                Some((_, Ok(m))) => Some(m),
                Some((raw, Err(why))) => {
                    eprintln!("Error: --mode {raw:?} {why}");
                    return Ok(2);
                }
            };
            check(CheckArgs {
                message,
                session,
                dry_run,
                json,
                budget: resolve_budget(budget),
                mode,
                pastes,
                from,
                explain,
            })
        }
        DoorsCommands::Stats { since, json } => stats(since.as_deref(), json),
        DoorsCommands::Reset { session } => reset(session.as_deref()),
        DoorsCommands::Mode { mode: m, session } => mode(m, session),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_builds_the_envelope_the_hook_receives() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/doors/hook-paste.json"))
                .unwrap();
        let session = fixture["session_id"].as_str().unwrap();
        let prompt = fixture["prompt"].as_str().unwrap();
        assert_eq!(paste_id(session), "11e5");
        let walden = include_str!("../../tests/fixtures/doors/pd/walden.txt");
        let composed = compose(
            "found this bit about huckleberries, worth a door? ",
            session,
            &[walden.to_string()],
            None,
        );
        assert_eq!(format!("{composed}\n\n keep it short"), prompt);
    }

    #[test]
    fn compose_wraps_from_in_a_channel_block() {
        assert_eq!(
            compose("hi", "s", &[], Some("@j:s")),
            "<channel source=\"cli\" user_id=\"@j:s\">\nhi\n</channel>"
        );
    }
}
