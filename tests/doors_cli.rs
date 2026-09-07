//! Integration tests for `mx doors` and the `mx kv` trigger surface.
//!
//! Drives the built binary against an isolated `MX_HOME` (harness mirrors
//! `tests/kv_set_cli.rs`). `mx doors` touches only flat files under `MX_HOME`,
//! so no serialization lock is needed and nothing reaches SurrealDB.
//!
//! The acceptance gate is `konkon_fires_from_a_matrix_channel_block`: a real
//! channel block, a door living on a NON-`doors` key, one line on stdout, one
//! fire row recorded.

use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.facts]
type = "list"

[keys.doors]
type = "list"

[keys.doors_fired]
type = "history"
max_entries = 5000
description = "Doors fire log = dedup table + telemetry."

[keys.doors_fired.data]
session = { type = "string", required = true }
key = { type = "string", required = true }
entry = { type = "string", required = true }
trigger = { type = "string" }
"#;

/// A real Matrix channel block: identity in the ATTRIBUTES, the actual message
/// in the body. `user="carmel"` must never open a `carmel` door on its own.
const CHANNEL_PROMPT: &str = concat!(
    r#"<channel source="matrix" chat_id="!r:s" message_id="$e" user="carmel" "#,
    r#"user_id="@j:s" room_name="delta">\ngood morning konkon\n</channel>"#
);

fn setup() -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), SCHEMA).unwrap();
    dir
}

fn mx_stdin(dir: &TempDir, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn mx");
    if let Some(s) = stdin {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to run mx")
}

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    mx_stdin(dir, args, Some(""))
}

fn ok(out: &std::process::Output, what: &str) -> String {
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

/// Push a door and return its kv id (the `kv-XXXXXX` hash without the prefix).
fn push_door(dir: &TempDir, key: &str, value: &str, extra: &[&str]) -> String {
    let mut args = vec!["kv", "push", key, value];
    args.extend_from_slice(extra);
    let out = ok(&mx(dir, &args), "kv push");
    out.trim()
        .split_whitespace()
        .next()
        .unwrap()
        .trim_start_matches("kv-")
        .to_string()
}

fn hook_input(session: &str, prompt: &str) -> String {
    serde_json::json!({
        "session_id": session,
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
    })
    .to_string()
}

fn fire_rows(dir: &TempDir) -> Vec<serde_json::Value> {
    let out = mx(
        dir,
        &["kv", "last", "doors_fired", "--count", "100", "--json"],
    );
    if !out.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The acceptance gate
// ---------------------------------------------------------------------------

#[test]
fn konkon_fires_from_a_matrix_channel_block() {
    let dir = setup();
    let id = push_door(
        &dir,
        "facts",
        "Carmel's everyday word for Q, the fox-sound.",
        &["--trigger", "konkon"],
    );

    let out = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", CHANNEL_PROMPT)),
    );
    let stdout = ok(&out, "doors hook");

    assert_eq!(
        stdout.trim(),
        format!(
            "\u{1f6aa} konkon \u{2192} Carmel's everyday word for Q, the fox-sound. (dig: facts/kv-{id})"
        ),
        "exactly one door line"
    );

    let rows = fire_rows(&dir);
    assert_eq!(rows.len(), 1, "one fire row: {rows:?}");
    let d = &rows[0]["data"];
    assert_eq!(d["session"], "s1");
    assert_eq!(d["key"], "facts");
    assert_eq!(d["entry"], id);
    assert_eq!(d["trigger"], "konkon");
}

#[test]
fn channel_attributes_do_not_open_a_door() {
    let dir = setup();
    push_door(&dir, "facts", "about carmel", &["--trigger", "carmel"]);
    let out = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", CHANNEL_PROMPT)),
    );
    assert_eq!(
        ok(&out, "doors hook"),
        "",
        "user=\"carmel\" in an attribute must not fire a carmel door"
    );
    assert!(fire_rows(&dir).is_empty());
}

// ---------------------------------------------------------------------------
// Dedup, sessions, budget
// ---------------------------------------------------------------------------

#[test]
fn same_session_fires_once_then_a_fresh_session_re_fires() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);

    let first = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "hi konkon")),
    );
    assert!(!ok(&first, "first hook").is_empty());
    assert_eq!(fire_rows(&dir).len(), 1);

    let second = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "konkon again")),
    );
    assert_eq!(ok(&second, "second hook"), "", "deduped within the session");
    assert_eq!(fire_rows(&dir).len(), 1, "no second row");

    let third = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s2", "hi konkon")),
    );
    assert!(
        !ok(&third, "third hook").is_empty(),
        "a fresh session re-fires"
    );
    assert_eq!(fire_rows(&dir).len(), 2);
}

#[test]
fn budget_caps_distinct_entries_and_defers_the_rest() {
    let dir = setup();
    for n in 1..=3 {
        push_door(
            &dir,
            "doors",
            &format!("door {n}"),
            &["--trigger", "konkon"],
        );
    }
    let out = mx_stdin(&dir, &["doors", "hook"], Some(&hook_input("s1", "konkon")));
    let stdout = ok(&out, "hook");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "default budget is 2: {lines:?}");
    assert_eq!(fire_rows(&dir).len(), 2, "only fired doors are recorded");

    // The deferred door was never recorded, so it is still eligible next prompt.
    let next = mx_stdin(&dir, &["doors", "hook"], Some(&hook_input("s1", "konkon")));
    assert_eq!(ok(&next, "hook").lines().count(), 1, "the deferred door");
}

#[test]
fn least_fired_door_wins_the_budget() {
    let dir = setup();
    let hot = push_door(&dir, "doors", "hot door", &["--trigger", "konkon"]);
    let cold = push_door(&dir, "doors", "cold door", &["--trigger", "konkon"]);

    // Burn three fires onto `hot` across distinct sessions.
    for n in 0..3 {
        ok(
            &mx(
                &dir,
                &[
                    "kv",
                    "push",
                    "doors_fired",
                    "konkon",
                    "--data",
                    &serde_json::json!({
                        "session": format!("burn{n}"),
                        "key": "doors",
                        "entry": hot,
                        "trigger": "konkon",
                    })
                    .to_string(),
                ],
            ),
            "seed fire",
        );
    }

    let out = mx_stdin(
        &dir,
        &["doors", "hook", "--budget", "1"],
        Some(&hook_input("s9", "konkon")),
    );
    let stdout = ok(&out, "hook");
    assert!(
        stdout.contains(&format!("kv-{cold}")),
        "the never-fired door wins: {stdout}"
    );
    assert!(!stdout.contains(&format!("kv-{hot}")));
}

// ---------------------------------------------------------------------------
// Safety contract: never a non-zero exit, never a stray write
// ---------------------------------------------------------------------------

#[test]
fn hook_always_exits_zero_on_every_failure_mode() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);

    let cases: Vec<(&str, Option<&str>)> = vec![
        ("malformed json", Some("{not json at all")),
        ("empty stdin", Some("")),
        ("json but not an object", Some("[1,2,3]")),
        ("object missing both fields", Some("{}")),
    ];
    for (what, stdin) in cases {
        let out = mx_stdin(&dir, &["doors", "hook"], stdin);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{what} must exit 0 — exit 2 would ERASE the prompt"
        );
        assert!(
            out.stdout.is_empty(),
            "{what} must print nothing to stdout: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    // Missing MX_CURRENT_AGENT: kv cannot resolve a store at all.
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    let out = cmd
        .args(["doors", "hook"])
        .env_remove("MX_CURRENT_AGENT")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin
                .as_mut()
                .unwrap()
                .write_all(hook_input("s1", "hi konkon").as_bytes())?;
            drop(c.stdin.take());
            c.wait_with_output()
        })
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "missing agent must still exit 0"
    );
    assert!(out.stdout.is_empty());
    assert!(
        !out.stderr.is_empty(),
        "the failure must be reported on stderr"
    );
}

#[test]
fn no_match_is_silent_and_writes_nothing() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);
    let before = std::fs::metadata(data_path(&dir))
        .unwrap()
        .modified()
        .unwrap();

    let out = mx_stdin(&dir, &["doors", "hook"], Some(&hook_input("s1", "hello")));
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "no match means empty stdout");

    let after = std::fs::metadata(data_path(&dir))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        before, after,
        "a no-match prompt must not write the kv file"
    );
}

#[test]
fn dry_run_prints_without_recording() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);

    let out = mx_stdin(
        &dir,
        &["doors", "hook", "--dry-run"],
        Some(&hook_input("s1", "hi konkon")),
    );
    assert!(!ok(&out, "dry-run hook").is_empty(), "dry-run still prints");
    assert!(fire_rows(&dir).is_empty(), "dry-run records nothing");

    let real = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "hi konkon")),
    );
    assert!(
        !ok(&real, "real hook").is_empty(),
        "the door is still eligible"
    );
    assert_eq!(fire_rows(&dir).len(), 1);
}

// ---------------------------------------------------------------------------
// Pointers, fragments, and `doors check`
// ---------------------------------------------------------------------------

#[test]
fn dig_pointer_carries_the_memory_link() {
    let dir = setup();
    let id = push_door(
        &dir,
        "doors",
        "A real machine in the fleet.",
        &["--trigger", "slaptop", "--memory", "kn-02e73234"],
    );
    let out = mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "the slaptop")),
    );
    assert!(
        ok(&out, "hook").contains(&format!("(dig: doors/kv-{id}, kn-02e73234)")),
        "the kn- link rides along with the kv pointer"
    );
}

#[test]
fn authored_fragment_beats_the_first_line_rule() {
    let dir = setup();
    push_door(
        &dir,
        "doors",
        "line one of the value\nline two",
        &[
            "--trigger",
            "gerf",
            "--fragment",
            "Gerf is GEOFF, not a typo.",
        ],
    );
    push_door(
        &dir,
        "doors",
        "first line only\nsecond line ignored",
        &["--trigger", "slaptop"],
    );

    let authored = ok(
        &mx(&dir, &["doors", "check", "ask gerf", "--session", "a"]),
        "check",
    );
    assert!(
        authored.contains("Gerf is GEOFF, not a typo."),
        "{authored}"
    );
    assert!(!authored.contains("line one"));

    let derived = ok(
        &mx(&dir, &["doors", "check", "the slaptop", "--session", "b"]),
        "check",
    );
    assert!(derived.contains("first line only"), "{derived}");
    assert!(!derived.contains("second line"));
}

#[test]
fn check_matches_the_hook_and_reports_json() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);

    let via_check = ok(
        &mx(
            &dir,
            &[
                "doors",
                "check",
                "hi konkon",
                "--session",
                "s9",
                "--dry-run",
            ],
        ),
        "check",
    );
    let via_hook = ok(
        &mx_stdin(
            &dir,
            &["doors", "hook", "--dry-run"],
            Some(&hook_input("s9", "hi konkon")),
        ),
        "hook",
    );
    assert_eq!(via_check, via_hook, "both paths render the same line");

    let json = ok(
        &mx(
            &dir,
            &[
                "doors",
                "check",
                "hi konkon",
                "--session",
                "s9",
                "--json",
                "--dry-run",
            ],
        ),
        "check --json",
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["fired"].as_array().unwrap().len(), 1);
    assert_eq!(v["deferred"], 0);
    assert_eq!(v["fired"][0]["trigger"], "konkon");
}

#[test]
fn reset_clears_fires_for_one_session_or_all() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);
    for s in ["s1", "s2"] {
        mx_stdin(&dir, &["doors", "hook"], Some(&hook_input(s, "hi konkon")));
    }
    assert_eq!(fire_rows(&dir).len(), 2);

    ok(
        &mx(&dir, &["doors", "reset", "--session", "s1"]),
        "reset s1",
    );
    assert_eq!(fire_rows(&dir).len(), 1);

    ok(&mx(&dir, &["doors", "reset"]), "reset all");
    assert!(fire_rows(&dir).is_empty());
}

#[test]
fn stats_counts_fires_and_names_doors_never_opened() {
    let dir = setup();
    push_door(&dir, "facts", "the fox-sound", &["--trigger", "konkon"]);
    let quiet = push_door(&dir, "doors", "never used", &["--trigger", "slaptop"]);
    mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "hi konkon")),
    );

    let json = ok(&mx(&dir, &["doors", "stats", "--json"]), "stats");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["total_fires"], 1);
    assert_eq!(v["triggers"][0]["trigger"], "konkon");
    let never = v["never_fired"].as_array().unwrap();
    assert_eq!(never.len(), 1);
    assert_eq!(never[0]["entry"], quiet);
}

// ---------------------------------------------------------------------------
// The kv authoring surface
// ---------------------------------------------------------------------------

#[test]
fn push_normalizes_and_dedupes_trigger_flags() {
    let dir = setup();
    push_door(
        &dir,
        "facts",
        "x",
        &[
            "--trigger",
            "  Kon Kon ",
            "--trigger",
            "KON KON",
            "--trigger",
            "konkon",
        ],
    );
    let json = ok(&mx(&dir, &["kv", "triggers", "--json"]), "kv triggers");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        v[0]["triggers"],
        serde_json::json!(["kon kon", "konkon"]),
        "normalized, deduped, order preserved"
    );
}

#[test]
fn update_replaces_clears_triggers_and_fragment() {
    let dir = setup();
    let id = push_door(
        &dir,
        "facts",
        "first line\nsecond",
        &["--trigger", "alpha", "--fragment", "authored one"],
    );
    let kv_id = format!("kv-{id}");

    ok(
        &mx(
            &dir,
            &["kv", "update", "facts", "--id", &kv_id, "--trigger", "beta"],
        ),
        "replace triggers",
    );
    let v: serde_json::Value =
        serde_json::from_str(&ok(&mx(&dir, &["kv", "triggers", "--json"]), "t")).unwrap();
    assert_eq!(v[0]["triggers"], serde_json::json!(["beta"]));
    assert_eq!(v[0]["fragment"], "authored one");

    // Clearing the fragment resumes the first-line rule.
    ok(
        &mx(
            &dir,
            &["kv", "update", "facts", "--id", &kv_id, "--fragment", ""],
        ),
        "clear fragment",
    );
    let v: serde_json::Value =
        serde_json::from_str(&ok(&mx(&dir, &["kv", "triggers", "--json"]), "t")).unwrap();
    assert_eq!(v[0]["fragment"], "first line");
    assert!(v[0]["authored_fragment"].is_null());

    // Clearing the triggers removes the door entirely.
    ok(
        &mx(
            &dir,
            &["kv", "update", "facts", "--id", &kv_id, "--trigger", ""],
        ),
        "clear triggers",
    );
    let v: serde_json::Value =
        serde_json::from_str(&ok(&mx(&dir, &["kv", "triggers", "--json"]), "t")).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0, "no doors remain");
}

#[test]
fn kv_triggers_lists_across_keys_and_filters_by_key() {
    let dir = setup();
    push_door(&dir, "facts", "a", &["--trigger", "konkon"]);
    push_door(&dir, "doors", "b", &["--trigger", "gerf"]);
    push_door(&dir, "doors", "c", &["--trigger", "slaptop"]);
    push_door(&dir, "doors", "d", &["--trigger", "ayo-"]);
    mx_stdin(
        &dir,
        &["doors", "hook"],
        Some(&hook_input("s1", "hi konkon")),
    );

    let all: serde_json::Value =
        serde_json::from_str(&ok(&mx(&dir, &["kv", "triggers", "--json"]), "all")).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 4);
    let keys: std::collections::HashSet<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 2, "entries span both keys");

    let konkon = all
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == "facts")
        .unwrap();
    assert_eq!(konkon["fires"], 1, "fire counts come from the log");

    let doors_only: serde_json::Value = serde_json::from_str(&ok(
        &mx(&dir, &["kv", "triggers", "doors", "--json"]),
        "one",
    ))
    .unwrap();
    assert_eq!(doors_only.as_array().unwrap().len(), 3);

    let text = ok(&mx(&dir, &["kv", "triggers"]), "plain");
    assert!(text.contains("facts") && text.contains("doors"));
    assert!(text.contains("fires=1"));
}

#[test]
fn plain_entries_are_untouched_by_the_new_columns() {
    let dir = setup();
    push_door(&dir, "facts", "no door here", &[]);
    let raw = std::fs::read_to_string(data_path(&dir)).unwrap();
    assert!(
        !raw.contains("triggers"),
        "absent fields stay absent: {raw}"
    );
    assert!(!raw.contains("fragment"));

    let out = ok(&mx(&dir, &["kv", "triggers"]), "kv triggers");
    assert!(out.is_empty(), "nothing carries triggers");
}
