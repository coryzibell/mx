//! Integration tests for doors P3 (#489): the `doors_people` map, the
//! `agent:` reader axis, pre-burn on authoring, `--expires`, and the
//! shared-trigger warning.
//!
//! Every run pins `MX_KV_SCHEMA`, `MX_KV_DATA` and `HOME` to the tempdir. kv
//! resolves its files by `MX_CURRENT_AGENT`, and when the per-agent file under
//! `MX_HOME` is missing it falls back to `~/.crewu/kv/<agent>.*`. These tests
//! run as `queue` on purpose (Q's main thread), so without explicit paths they
//! would read and write the live store. `CLAUDE_CODE_SESSION_ID` is removed
//! unless a test sets it. Fixtures are synthetic.

use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.facts]
type = "list"

[keys.log]
type = "history"

[keys.doors_people]
type = "state"

[keys.doors_session]
type = "history"

[keys.doors_session.data]
session = { type = "string", required = true }
event = { type = "string", required = true }
mode = { type = "string" }
by = { type = "string" }
kind = { type = "string" }

[keys.doors_fired]
type = "history"
max_entries = 5000

[keys.doors_fired.data]
session = { type = "string", required = true }
key = { type = "string", required = true }
entry = { type = "string", required = true }
trigger = { type = "string" }
origin = { type = "string" }
place = { type = "string" }
reader = { type = "string" }
"#;

fn setup() -> TempDir {
    setup_schema(SCHEMA)
}

fn setup_schema(schema: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("store.toml"), schema).unwrap();
    dir
}

fn data_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("kv").join("data").join("store.json")
}

fn run(dir: &TempDir, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("HOME", dir.path())
        .env(
            "MX_KV_SCHEMA",
            dir.path().join("kv").join("schema").join("store.toml"),
        )
        .env("MX_KV_DATA", data_path(dir))
        .env("MX_CURRENT_AGENT", "queue")
        .env_remove("MX_DOORS_NAMES")
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

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    run(dir, args, "", &[])
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

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Push a door onto `facts` and return its id.
fn push(dir: &TempDir, value: &str, extra: &[&str]) -> String {
    push_to(dir, "facts", value, extra)
}

fn push_to(dir: &TempDir, key: &str, value: &str, extra: &[&str]) -> String {
    let mut args = vec!["kv", "push", key, value];
    args.extend_from_slice(extra);
    let out = ok(&mx(dir, &args), "kv push");
    out.split_whitespace()
        .next()
        .unwrap()
        .trim_start_matches("kv-")
        .to_string()
}

fn map(dir: &TempDir, user_id: &str, json: &str) {
    ok(
        &mx(dir, &["kv", "set", "doors_people", user_id, json]),
        "kv set doors_people",
    );
}

fn hook_with(
    dir: &TempDir,
    session: &str,
    prompt: &str,
    extra: serde_json::Value,
) -> std::process::Output {
    let mut input = serde_json::json!({
        "session_id": session,
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
    });
    if let serde_json::Value::Object(m) = extra {
        for (k, v) in m {
            input[k] = v;
        }
    }
    run(dir, &["doors", "hook"], &input.to_string(), &[])
}

fn hook(dir: &TempDir, session: &str, prompt: &str) -> String {
    ok(
        &hook_with(dir, session, prompt, serde_json::json!({})),
        "doors hook",
    )
}

fn channel(user_id: &str, body: &str) -> String {
    format!("<channel source=\"matrix\" user=\"alice\" user_id=\"{user_id}\">\n{body}\n</channel>")
}

fn fire_rows(dir: &TempDir) -> Vec<serde_json::Value> {
    let out = mx(
        dir,
        &["kv", "last", "doors_fired", "--count", "1000", "--json"],
    );
    if !out.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

/// The ids in a hook's output, in print order.
fn fired_ids(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| l.rsplit("/kv-").next())
        .map(|s| s.trim_end_matches(')').to_string())
        .collect()
}

fn explain(dir: &TempDir, message: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["doors", "check", message, "--explain", "--json"];
    args.extend_from_slice(extra);
    serde_json::from_str(&ok(&mx(dir, &args), "check --explain")).unwrap()
}

// ---------------------------------------------------------------------------
// Item 1: doors_people and channel routing
// ---------------------------------------------------------------------------

struct Senders {
    house: String,
    alice: String,
    bob: String,
    mallory: String,
}

fn senders(dir: &TempDir) -> Senders {
    map(
        dir,
        "@alice:example.org",
        r#"{"person":"alice","house":true}"#,
    );
    map(dir, "@bob:example.org", r#"{"person":"bob","house":false}"#);
    Senders {
        house: push(dir, "house door", &["--trigger", "widget"]),
        alice: push(
            dir,
            "alice door",
            &["--trigger", "widget", "--scope", "person:alice"],
        ),
        bob: push(
            dir,
            "bob door",
            &["--trigger", "widget", "--scope", "person:bob"],
        ),
        mallory: push(
            dir,
            "mallory door",
            &["--trigger", "widget", "--scope", "person:mallory"],
        ),
    }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn a_mapped_house_sender_opens_house_and_their_person_doors() {
    let dir = setup();
    let s = senders(&dir);
    let out = hook(&dir, "s1", &channel("@alice:example.org", "widget"));
    assert_eq!(
        sorted(fired_ids(&out)),
        sorted(vec![s.house, s.alice]),
        "{out}"
    );
    let rows = fire_rows(&dir);
    let mut places: Vec<&str> = rows
        .iter()
        .map(|r| r["data"]["place"].as_str().unwrap())
        .collect();
    places.sort();
    assert_eq!(places, vec!["house", "person:alice"]);
}

#[test]
fn a_mapped_non_house_sender_opens_only_their_person_doors() {
    let dir = setup();
    let s = senders(&dir);
    let out = hook(&dir, "s1", &channel("@bob:example.org", "widget"));
    assert_eq!(fired_ids(&out), vec![s.bob]);
}

#[test]
fn an_unmapped_sender_never_opens_a_house_door() {
    let dir = setup();
    let s = senders(&dir);
    let out = hook(&dir, "s1", &channel("@mallory:example.org", "widget"));
    assert_eq!(fired_ids(&out), vec![s.mallory]);
}

#[test]
fn with_no_map_every_channel_sender_is_unmapped() {
    let dir = setup();
    let house = push(&dir, "house door", &["--trigger", "widget"]);
    assert_eq!(
        hook(&dir, "s1", &channel("@alice:example.org", "widget")),
        ""
    );
    assert!(fire_rows(&dir).is_empty());
    let v = explain(&dir, "widget", &["--from", "@alice:example.org"]);
    assert_eq!(v["segments"][0]["mapped"], false);
    assert_eq!(
        v["segments"][0]["places"],
        serde_json::json!(["person:alice"])
    );
    assert_eq!(v["doors"][0]["id"], house.as_str());
    assert_eq!(v["doors"][0]["status"], "HELD");
}

#[test]
fn a_malformed_user_id_opens_only_pinned() {
    let dir = setup();
    senders(&dir);
    let pin = push(
        &dir,
        "pinned door",
        &["--trigger", "widget", "--scope", "pinned"],
    );
    for uid in [
        "",
        "mallory",
        "@mallory",
        "@:example.org",
        "@mal/lory:example.org",
    ] {
        let session = format!("s-{uid}");
        let out = hook(&dir, &session, &channel(uid, "widget"));
        assert_eq!(fired_ids(&out), vec![pin.clone()], "{uid:?}: {out}");
    }
}

#[test]
fn a_spoofed_display_name_from_an_unmapped_user_id_does_not_resolve() {
    let dir = setup();
    let s = senders(&dir);
    // `user="alice"` names a mapped person; the user_id is mallory's.
    let prompt = concat!(
        r#"<channel source="matrix" user="alice" display_name="alice" "#,
        r#"user_id="@mallory:example.org">"#,
        "\nwidget\n</channel>"
    );
    let out = hook(&dir, "s1", prompt);
    assert_eq!(fired_ids(&out), vec![s.mallory], "{out}");
}

#[test]
fn a_bad_map_entry_is_unmapped_and_silent() {
    let dir = setup();
    let house = push(&dir, "house door", &["--trigger", "widget"]);
    map(&dir, "@alice:example.org", "not json");
    map(&dir, "@bob:example.org", r#"{"person":"Bob","house":true}"#);
    for uid in ["@alice:example.org", "@bob:example.org"] {
        let out = hook_with(&dir, uid, &channel(uid, "widget"), serde_json::json!({}));
        assert_eq!(ok(&out, "hook"), "", "{uid}");
        assert_eq!(stderr(&out), "", "no error, no output: {uid}");
    }
    let v = explain(&dir, "widget", &[]);
    assert_eq!(v["fired"][0]["id"], house.as_str(), "typed text unaffected");
}

#[test]
fn explain_text_shows_each_channel_resolution() {
    let dir = setup();
    senders(&dir);
    let out = ok(
        &mx(
            &dir,
            &[
                "doors",
                "check",
                "widget",
                "--from",
                "@alice:example.org",
                "--explain",
            ],
        ),
        "explain",
    );
    assert!(
        out.contains("channel(@alice:example.org mapped)")
            && out.contains("places {house, person:alice}"),
        "{out}"
    );
    let out = ok(
        &mx(
            &dir,
            &[
                "doors",
                "check",
                "widget",
                "--from",
                "@mallory:example.org",
                "--explain",
            ],
        ),
        "explain",
    );
    assert!(
        out.contains("channel(@mallory:example.org unmapped)")
            && out.contains("places {person:mallory}"),
        "{out}"
    );
    let v = explain(&dir, "widget", &["--from", "@alice:example.org"]);
    assert_eq!(v["segments"][0]["user_id"], "@alice:example.org");
    assert_eq!(v["segments"][0]["mapped"], true);
    assert_eq!(
        v["segments"][0]["places"],
        serde_json::json!(["house", "person:alice"])
    );
    let v = explain(&dir, "widget", &[]);
    assert_eq!(v["segments"][0]["mapped"], serde_json::Value::Null);
}

#[test]
fn typed_text_gains_its_owner_in_house_mode() {
    let dir = setup();
    let s = senders(&dir);
    assert_eq!(
        fired_ids(&hook(&dir, "s0", "widget")),
        vec![s.house.clone()]
    );
    map(
        &dir,
        "@alice:example.org",
        r#"{"person":"alice","house":true,"owner":true}"#,
    );
    let out = hook(&dir, "s1", "widget");
    assert_eq!(sorted(fired_ids(&out)), sorted(vec![s.house, s.alice]));
}

#[test]
fn typed_text_gains_its_owner_in_book_mode() {
    let dir = setup();
    let house = push(&dir, "house door", &["--trigger", "widget"]);
    let alice = push(
        &dir,
        "alice door",
        &["--trigger", "widget", "--scope", "person:alice"],
    );
    let walden = push(
        &dir,
        "walden door",
        &["--trigger", "widget", "--scope", "book:walden"],
    );
    let book = ["--mode", "book:walden"];
    let v = explain(&dir, "widget", &book);
    assert_eq!(
        v["segments"][0]["places"],
        serde_json::json!(["book:walden"])
    );
    assert_eq!(v["fired"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["fired"][0]["id"], walden.as_str());

    map(
        &dir,
        "@alice:example.org",
        r#"{"person":"alice","owner":true}"#,
    );
    let v = explain(&dir, "widget", &[&book[..], &["--budget", "1"]].concat());
    assert_eq!(
        v["segments"][0]["places"],
        serde_json::json!(["book:walden", "person:alice"])
    );
    let status = |id: &str| {
        v["doors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .map(|d| (d["status"].clone(), d["pool"].clone()))
            .unwrap()
    };
    assert_eq!(status(&walden), ("FIRED".into(), "book".into()));
    assert_eq!(
        status(&alice),
        ("FIRED".into(), "house".into()),
        "a person hit spends the house pool, so budget 1 fires both"
    );
    assert_eq!(status(&house).0, "HELD");
}
