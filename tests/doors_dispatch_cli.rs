//! Integration tests for `mx doors dispatch`, the PreToolUse(Agent) entry point.
//!
//! Drives the built binary against an isolated `MX_HOME`, one process at a time,
//! like `tests/doors_cli.rs`. The harness store resolves through
//! `MX_CURRENT_AGENT=test`. A real hook command sets that variable too: it picks
//! the store, never the reader.

use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const FIRED_NO_ORIGIN: &str = r#"
[keys.facts]
type = "list"

[keys.doors_fired]
type = "history"
max_entries = 5000

[keys.doors_fired.data.session]
type = "string"
required = true

[keys.doors_fired.data.key]
type = "string"
required = true

[keys.doors_fired.data.entry]
type = "string"
required = true

[keys.doors_fired.data.trigger]
type = "string"
"#;

const ORIGIN_AND_SESSION: &str = r#"
[keys.doors_fired.data.origin]
type = "string"

[keys.doors_fired.data.place]
type = "string"

[keys.doors_fired.data.reader]
type = "string"

[keys.doors_session]
type = "history"
max_entries = 1000

[keys.doors_session.data.session]
type = "string"
required = true

[keys.doors_session.data.event]
type = "string"
required = true

[keys.doors_session.data.mode]
type = "string"

[keys.doors_session.data.by]
type = "string"

[keys.doors_session.data.kind]
type = "string"
"#;

fn setup_with(schema: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), schema).unwrap();
    dir
}

fn setup() -> TempDir {
    setup_with(&format!("{FIRED_NO_ORIGIN}{ORIGIN_AND_SESSION}"))
}

fn mx_env(dir: &TempDir, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .env_remove("MX_DOORS_BUDGET")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn mx");
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to run mx")
}

fn mx(dir: &TempDir, args: &[&str]) -> Output {
    mx_env(dir, args, "", &[])
}

fn ok(out: &Output, what: &str) -> String {
    assert!(
        out.status.success(),
        "{what} failed ({:?}): {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn data_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("kv").join("data").join("test.json")
}

fn push_door(dir: &TempDir, value: &str, extra: &[&str]) -> String {
    let mut args = vec!["kv", "push", "facts", value];
    args.extend_from_slice(extra);
    ok(&mx(dir, &args), "kv push")
        .split_whitespace()
        .next()
        .unwrap()
        .trim_start_matches("kv-")
        .to_string()
}

fn agent_door(dir: &TempDir, agent: &str) -> String {
    let scope = format!("agent:{agent}");
    push_door(
        dir,
        "A fragment for the widget door.",
        &["--trigger", "widget", "--scope", &scope],
    )
}

fn payload(session: &str, tool_use_id: &str, agent: &str, prompt: &str) -> Value {
    json!({
        "session_id": session,
        "transcript_path": "/tmp/t.jsonl",
        "cwd": "/tmp",
        "prompt_id": "p1",
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Agent",
        "tool_input": {
            "description": "a task",
            "prompt": prompt,
            "subagent_type": agent,
            "run_in_background": false,
        },
        "tool_use_id": tool_use_id,
    })
}

fn dispatch(dir: &TempDir, input: &Value) -> Output {
    mx_env(dir, &["doors", "dispatch"], &input.to_string(), &[])
}

/// Exit 0 and empty stdout, the shape of every "nothing".
fn assert_nothing(out: &Output, what: &str) {
    assert_eq!(out.status.code(), Some(0), "{what}");
    assert!(
        out.stdout.is_empty(),
        "{what}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).lines().count() <= 1,
        "{what}: at most one stderr line: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The parsed stdout of a dispatch that fired. Asserts it is one JSON line.
fn fired_json(out: &Output) -> Value {
    let stdout = ok(out, "dispatch");
    assert_eq!(stdout.lines().count(), 1, "exactly one line: {stdout}");
    serde_json::from_str(&stdout).expect("stdout is JSON")
}

fn updated_input(out: &Output) -> Value {
    fired_json(out)["hookSpecificOutput"]["updatedInput"].clone()
}

fn fire_rows(dir: &TempDir) -> Vec<Value> {
    let out = mx(
        dir,
        &["kv", "last", "doors_fired", "--count", "100", "--json"],
    );
    if !out.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

fn hook_as(dir: &TempDir, session: &str, agent: &str, prompt: &str, args: &[&str]) -> String {
    let input = json!({
        "session_id": session,
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
        "agent_type": agent,
    });
    let mut all = vec!["doors", "hook"];
    all.extend_from_slice(args);
    ok(&mx_env(dir, &all, &input.to_string(), &[]), "doors hook")
}

fn stats_json(dir: &TempDir) -> Value {
    serde_json::from_str(&ok(&mx(dir, &["doors", "stats", "--json"]), "stats")).unwrap()
}

fn has_key(v: &Value, name: &str) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, c)| k == name || has_key(c, name)),
        Value::Array(a) => a.iter().any(|c| has_key(c, name)),
        _ => false,
    }
}

#[test]
fn a_matching_agent_door_lands_after_the_prompt() {
    let dir = setup();
    let id = agent_door(&dir, "helper");
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_eq!(
        updated_input(&out)["prompt"],
        format!(
            "fix the widget\n\n\u{1f6aa} widget \u{2192} A fragment for the widget door. (dig: facts/kv-{id})"
        )
    );
}

#[test]
fn a_non_matching_agent_gets_nothing() {
    let dir = setup();
    agent_door(&dir, "helper");
    let out = dispatch(&dir, &payload("s1", "t1", "reviewer", "fix the widget"));
    assert_nothing(&out, "other agent");
    assert!(fire_rows(&dir).is_empty());
}

#[test]
fn no_door_at_all_saves_nothing() {
    let dir = setup();
    ok(&mx(&dir, &["kv", "push", "facts", "plain"]), "kv push");
    let path = data_path(&dir);
    let before = std::fs::read(&path).unwrap();
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&out, "no door");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[test]
fn the_rest_of_tool_input_passes_through() {
    let dir = setup();
    agent_door(&dir, "helper");
    let tool_input = json!({
        "description": "a task",
        "prompt": "fix the widget",
        "subagent_type": "helper",
        "run_in_background": true,
        "name": "worker-1",
        "model": "sonnet",
        "invented_field": {"nested": [1, {"deep": null}], "flag": false},
    });
    let mut input = payload("s1", "t1", "helper", "");
    input["tool_input"] = tool_input.clone();
    let mut updated = updated_input(&dispatch(&dir, &input));
    assert_ne!(updated["prompt"], tool_input["prompt"]);
    updated["prompt"] = tool_input["prompt"].clone();
    assert_eq!(updated, tool_input);
}

#[test]
fn the_same_tool_use_id_fires_once_and_a_new_one_fires_again() {
    let dir = setup();
    agent_door(&dir, "helper");
    let first = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    fired_json(&first);
    let again = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&again, "same tool_use_id");
    assert_eq!(fire_rows(&dir).len(), 1);
    let next = dispatch(&dir, &payload("s1", "t2", "helper", "fix the widget"));
    fired_json(&next);
    assert_eq!(fire_rows(&dir).len(), 2);
}

#[test]
fn a_malformed_payload_is_nothing() {
    let dir = setup();
    agent_door(&dir, "helper");
    let good = payload("s1", "t1", "helper", "fix the widget");
    let edited = |f: &dyn Fn(&mut Value)| {
        let mut v = good.clone();
        f(&mut v);
        v.to_string()
    };
    let cases = [
        ("not JSON", "{not json".to_string()),
        ("empty stdin", String::new()),
        (
            "tool_input missing",
            edited(&|v| {
                v.as_object_mut().unwrap().remove("tool_input");
            }),
        ),
        (
            "tool_input a string",
            edited(&|v| v["tool_input"] = json!("fix the widget")),
        ),
        (
            "prompt a number",
            edited(&|v| v["tool_input"]["prompt"] = json!(7)),
        ),
        (
            "subagent_type missing",
            edited(&|v| {
                v["tool_input"]
                    .as_object_mut()
                    .unwrap()
                    .remove("subagent_type");
            }),
        ),
        (
            "tool_use_id missing",
            edited(&|v| {
                v.as_object_mut().unwrap().remove("tool_use_id");
            }),
        ),
    ];
    for (what, input) in cases {
        let out = mx_env(&dir, &["doors", "dispatch"], &input, &[]);
        assert_nothing(&out, what);
    }
    assert!(fire_rows(&dir).is_empty());
}

#[test]
fn house_and_pinned_doors_without_an_agent_do_not_reach_a_child() {
    let dir = setup();
    push_door(&dir, "A house door.", &["--trigger", "widget"]);
    push_door(
        &dir,
        "A pinned door.",
        &["--trigger", "widget", "--scope", "pinned"],
    );
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&out, "house and pinned");
}

#[test]
fn a_nested_dispatch_reads_the_child_not_the_dispatcher() {
    let dir = setup();
    agent_door(&dir, "helper");

    let mut held = payload("s1", "t1", "reviewer", "fix the widget");
    held["agent_type"] = json!("helper");
    held["agent_id"] = json!("a1");
    assert_nothing(&dispatch(&dir, &held), "dispatcher is the door's agent");

    let mut fires = payload("s1", "t2", "helper", "fix the widget");
    fires["agent_type"] = json!("reviewer");
    fires["agent_id"] = json!("a2");
    fired_json(&dispatch(&dir, &fires));
}

#[test]
fn the_current_agent_is_not_the_reader() {
    let dir = setup();
    agent_door(&dir, "test");
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&out, "MX_CURRENT_AGENT names the door's agent");
}

#[test]
fn stdout_carries_no_permission_decision() {
    let dir = setup();
    agent_door(&dir, "helper");
    let v = fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    assert!(!has_key(&v, "permissionDecision"), "{v}");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
}

#[test]
fn a_fire_row_names_the_dispatch_the_child_and_the_root_session() {
    let dir = setup();
    let id = agent_door(&dir, "helper");
    let mut input = payload("root", "toolu_9", "helper", "fix the widget");
    input["agent_type"] = json!("reviewer");
    fired_json(&dispatch(&dir, &input));
    let rows = fire_rows(&dir);
    assert_eq!(rows.len(), 1);
    let d = &rows[0]["data"];
    assert_eq!(d["origin"], "dispatch:toolu_9");
    assert_eq!(d["reader"], "helper");
    assert_eq!(d["session"], "root");
    assert_eq!(d["entry"], id);
    assert_eq!(d["place"], "house");
}

#[test]
fn a_dispatch_fire_does_not_burn_the_door_on_the_main_thread() {
    let dir = setup();
    agent_door(&dir, "helper");
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    let line = hook_as(&dir, "s1", "helper", "fix the widget", &[]);
    assert!(line.contains("widget"), "main path still fires: {line:?}");
}

#[test]
fn a_dispatch_fire_does_not_shift_main_thread_ordering() {
    let dir = setup();
    let a = agent_door(&dir, "helper");
    let b = agent_door(&dir, "helper");
    let winner = |text: &str| {
        if text.contains(&format!("kv-{a}")) {
            a.clone()
        } else {
            assert!(text.contains(&format!("kv-{b}")), "{text}");
            b.clone()
        }
    };

    let once = mx_env(
        &dir,
        &["doors", "dispatch", "--budget", "1"],
        &payload("s1", "t1", "helper", "fix the widget").to_string(),
        &[],
    );
    let dispatched = winner(&ok(&once, "dispatch"));

    let main = hook_as(&dir, "s2", "helper", "fix the widget", &["--budget", "1"]);
    assert_eq!(winner(&main), dispatched, "ordering ignores dispatch rows");

    let twice = mx_env(
        &dir,
        &["doors", "dispatch", "--budget", "1"],
        &payload("s3", "t2", "helper", "fix the widget").to_string(),
        &[],
    );
    assert_ne!(
        winner(&ok(&twice, "dispatch")),
        dispatched,
        "a dispatch ranks on the dispatch book"
    );
}

#[test]
fn an_authored_pre_burn_does_not_hold_a_dispatch() {
    let dir = setup();
    let out = mx_env(
        &dir,
        &[
            "kv",
            "push",
            "facts",
            "A fragment.",
            "--trigger",
            "widget",
            "--scope",
            "agent:helper",
        ],
        "",
        &[("CLAUDE_CODE_SESSION_ID", "s1")],
    );
    ok(&out, "kv push");
    let rows = fire_rows(&dir);
    assert_eq!(rows[0]["data"]["origin"], "authored");
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
}

#[test]
fn a_dispatch_row_does_not_stop_a_pre_burn() {
    let dir = setup();
    let id = agent_door(&dir, "helper");
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    let out = mx_env(
        &dir,
        &[
            "kv",
            "update",
            "facts",
            "A newer fragment.",
            "--id",
            &format!("kv-{id}"),
            "--trigger",
            "widget",
        ],
        "",
        &[("CLAUDE_CODE_SESSION_ID", "s1")],
    );
    ok(&out, "kv update");
    let rows = fire_rows(&dir);
    assert!(
        rows.iter().any(|r| r["data"]["origin"] == "authored"),
        "{rows:?}"
    );
    let line = hook_as(&dir, "s1", "helper", "fix the widget", &[]);
    assert_eq!(line, "", "the author holds the door it just wrote");
}

#[test]
fn stats_separate_dispatch_from_typed() {
    let dir = setup();
    let typed = push_door(&dir, "A house door.", &["--trigger", "sprocket"]);
    let child = agent_door(&dir, "helper");
    hook_as(&dir, "s1", "lead", "a sprocket", &[]);
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t2", "helper", "fix the widget"),
    ));

    let v = stats_json(&dir);
    assert_eq!(v["total_fires"], 3, "dispatch fires stay in the total");
    let mut origins = v["origins"].as_array().unwrap().clone();
    origins.sort_by_key(|o| o["origin"].as_str().unwrap().to_string());
    assert_eq!(
        origins,
        vec![
            json!({"origin": "dispatch", "fires": 2}),
            json!({"origin": "typed", "fires": 1}),
        ]
    );
    let entry = |id: &str| {
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entry"] == id)
            .map(|e| e["fires"].clone())
    };
    assert_eq!(entry(&typed), Some(json!(1)));
    assert_eq!(entry(&child), Some(json!(2)));
    assert!(
        v["never_fired"].as_array().unwrap().is_empty(),
        "a dispatch-only door has fired: {v}"
    );

    let text = ok(&mx(&dir, &["doors", "stats"]), "stats");
    let section: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "by origin:")
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(str::trim)
        .collect();
    let split: Vec<Vec<&str>> = section
        .iter()
        .map(|l| l.split_whitespace().collect())
        .collect();
    assert_eq!(split, vec![vec!["2", "dispatch"], vec!["1", "typed"]]);
}

#[test]
fn stats_since_narrows_origins_too() {
    let dir = setup();
    agent_door(&dir, "helper");
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    let path = data_path(&dir);
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut data: Value = serde_json::from_str(&raw).unwrap();
    let mut aged = false;
    age_rows(&mut data, &mut aged);
    assert!(aged, "found the fire row: {raw}");
    std::fs::write(&path, data.to_string()).unwrap();

    let out = mx(&dir, &["doors", "stats", "--since", "1h", "--json"]);
    let v: Value = serde_json::from_str(&ok(&out, "stats --since")).unwrap();
    assert_eq!(v["total_fires"], 0);
    assert_eq!(v["origins"], json!([]));
    assert_eq!(stats_json(&dir)["origins"][0]["fires"], 1);
}

/// Backdate every `doors_fired` row in a raw data file.
fn age_rows(v: &mut Value, aged: &mut bool) {
    match v {
        Value::Object(m) => {
            if m.get("data").is_some_and(|d| d.get("trigger").is_some()) && m.contains_key("ts") {
                m["ts"] = json!("2020-01-01T00:00:00+00:00");
                *aged = true;
            }
            m.values_mut().for_each(|c| age_rows(c, aged));
        }
        Value::Array(a) => a.iter_mut().for_each(|c| age_rows(c, aged)),
        _ => {}
    }
}

#[test]
fn a_schema_without_origin_fires_nothing() {
    let dir = setup_with(FIRED_NO_ORIGIN);
    agent_door(&dir, "helper");
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&out, "no origin field");
    assert!(!out.stderr.is_empty(), "the skip says why");
    assert!(fire_rows(&dir).is_empty());
}

#[test]
fn an_unavailable_kv_is_nothing() {
    let dir = TempDir::new().unwrap();
    let out = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&out, "no schema");
}

#[test]
fn a_long_plain_prose_prompt_still_fires() {
    let dir = setup();
    agent_door(&dir, "helper");
    let sentence = "The afternoon light settled over the valley while the river ran on. ";
    let prompt = format!("{}Now fix the widget.", sentence.repeat(20));
    assert!(prompt.len() > 600);
    let out = dispatch(&dir, &payload("s1", "t1", "helper", &prompt));
    let updated = updated_input(&out);
    assert!(
        updated["prompt"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{prompt}\n\n"))
    );
}

#[test]
fn dry_run_prints_and_writes_nothing() {
    let dir = setup();
    agent_door(&dir, "helper");
    let input = payload("s1", "t1", "helper", "fix the widget").to_string();
    let path = data_path(&dir);
    let before = std::fs::read(&path).unwrap();
    let out = mx_env(&dir, &["doors", "dispatch", "--dry-run"], &input, &[]);
    fired_json(&out);
    assert!(fire_rows(&dir).is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_task_tool_call_fires_too() {
    let dir = setup();
    agent_door(&dir, "helper");
    let mut input = payload("s1", "t1", "helper", "fix the widget");
    input["tool_name"] = json!("Task");
    fired_json(&dispatch(&dir, &input));
}

#[test]
fn book_mode_holds_a_house_door_and_pinned_still_fires() {
    let dir = setup();
    agent_door(&dir, "helper");
    let pinned = push_door(
        &dir,
        "A pinned door.",
        &[
            "--trigger",
            "sprocket",
            "--scope",
            "pinned",
            "--scope",
            "agent:helper",
        ],
    );
    ok(
        &mx(&dir, &["doors", "mode", "book:walden", "--session", "s1"]),
        "set mode",
    );
    let house = dispatch(&dir, &payload("s1", "t1", "helper", "fix the widget"));
    assert_nothing(&house, "house door in book mode");
    let out = dispatch(&dir, &payload("s1", "t2", "helper", "fix the sprocket"));
    let prompt = updated_input(&out)["prompt"].as_str().unwrap().to_string();
    assert!(prompt.contains(&format!("kv-{pinned}")), "{prompt}");
}

#[test]
fn a_dispatch_writes_no_session_row() {
    let dir = setup();
    agent_door(&dir, "helper");
    fired_json(&dispatch(
        &dir,
        &payload("s1", "t1", "helper", "fix the widget"),
    ));
    let out = mx(&dir, &["kv", "last", "doors_session", "--json"]);
    let rows: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap_or_default();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn budget_zero_is_the_kill_switch() {
    let dir = setup();
    agent_door(&dir, "helper");
    let input = payload("s1", "t1", "helper", "fix the widget").to_string();
    let out = mx_env(&dir, &["doors", "dispatch", "--budget", "0"], &input, &[]);
    assert_nothing(&out, "budget 0");
    assert!(fire_rows(&dir).is_empty());
}
