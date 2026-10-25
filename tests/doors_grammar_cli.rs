//! Integration tests for the doors match grammar (#490): `p + q`, affix
//! wildcards, and the entry-level `--exclude` list.
//!
//! Drives the built binary against an isolated `MX_HOME`, harness mirroring
//! `tests/doors_cli.rs`. Nothing here reaches SurrealDB or the live kv. All
//! vocabulary is invented.

use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.facts]
type = "list"

[keys.doors_fired]
type = "history"
max_entries = 5000

[keys.doors_fired.data]
session = { type = "string", required = true }
key = { type = "string", required = true }
entry = { type = "string", required = true }
trigger = { type = "string" }
"#;

fn setup() -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), SCHEMA).unwrap();
    dir
}

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .env_remove("MX_DOORS_BUDGET")
        .stdin(Stdio::null())
        .output()
        .expect("failed to run mx")
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

fn push_door(dir: &TempDir, value: &str, extra: &[&str]) -> String {
    let mut args = vec!["kv", "push", "facts", value];
    args.extend_from_slice(extra);
    let out = ok(&mx(dir, &args), "kv push");
    out.split_whitespace()
        .next()
        .unwrap()
        .trim_start_matches("kv-")
        .to_string()
}

fn triggers_json(dir: &TempDir) -> serde_json::Value {
    serde_json::from_str(&ok(&mx(dir, &["kv", "triggers", "--json"]), "json")).unwrap()
}

fn update(dir: &TempDir, id: &str, extra: &[&str]) -> std::process::Output {
    let hash = format!("kv-{id}");
    let mut args = vec!["kv", "update", "facts", "--id", &hash];
    args.extend_from_slice(extra);
    mx(dir, &args)
}

#[test]
fn grammar_triggers_are_stored_and_shown_readably() {
    let dir = setup();
    let id = push_door(
        &dir,
        "zorbels herd quuxers",
        &[
            "--trigger",
            "Zorbel + Quuxer*",
            "--trigger",
            "frim_bo + *flimbing",
        ],
    );

    let text = ok(&mx(&dir, &["kv", "triggers"]), "kv triggers");
    assert_eq!(
        text,
        format!(
            "facts\n  kv-{id}  [zorbel + quuxer*, frim_bo + *flimbing]  fires=0\n      \
             \"frim_bo + *flimbing\" matches as \"frim bo + *flimbing\"\n      \
             zorbels herd quuxers\n"
        )
    );

    let v = triggers_json(&dir);
    assert_eq!(
        v[0]["triggers"],
        serde_json::json!(["zorbel + quuxer*", "frim_bo + *flimbing"])
    );
    assert_eq!(
        v[0]["matches_as"],
        serde_json::json!(["zorbel + quuxer*", "frim bo + *flimbing"])
    );
    assert!(v[0].get("exclude").is_none());
}

#[test]
fn plain_trigger_audit_output_is_unchanged() {
    let dir = setup();
    let id = push_door(&dir, "the snorp", &["--trigger", "snorp-"]);
    assert_eq!(
        ok(&mx(&dir, &["kv", "triggers"]), "kv triggers"),
        format!(
            "facts\n  kv-{id}  [snorp-]  fires=0\n      \"snorp-\" matches as \"snorp\"\n      the snorp\n"
        )
    );
    let raw =
        std::fs::read_to_string(dir.path().join("kv").join("data").join("test.json")).unwrap();
    assert!(!raw.contains("exclude"), "absent field stays absent: {raw}");
}

#[test]
fn each_grammar_rejection_exits_4_and_names_the_rule() {
    let dir = setup();
    for (flag, why) in [
        ("--trigger=zorbel +", "empty side of `+`"),
        ("--trigger=+ zorbel", "empty side of `+`"),
        ("--trigger=zorbel + + quuxer", "empty side of `+`"),
        ("--trigger=+", "empty side of `+`"),
        ("--trigger=*", "at least 5"),
        ("--trigger=zorb*", "at least 5"),
        ("--trigger=**zorbel", "more than one `*`"),
        ("--trigger=*zorbel*", "more than one `*`"),
        ("--trigger=zor*bel", "mid-word"),
        ("--trigger=*zorb-el", "letters and digits plus one `*`"),
        ("--trigger=--- + zorbel", "side 1 with no letters or digits"),
        ("--exclude=zorbel + quuxer", "an exclusion is one phrase"),
        ("--exclude=zor*bel", "mid-word"),
        ("--exclude=!!!", "could never hold the door"),
    ] {
        let out = mx(&dir, &["kv", "push", "facts", "x", "--trigger=frob", flag]);
        assert_eq!(out.status.code(), Some(4), "{flag:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(err.starts_with("Error: "), "{flag:?}: {err}");
        assert!(err.contains(why), "{flag:?} should say {why:?}: {err}");
    }
    assert!(ok(&mx(&dir, &["kv", "triggers"]), "kv triggers").is_empty());

    // The same rules guard update.
    let id = push_door(&dir, "x", &["--trigger", "frob"]);
    for flag in ["--trigger=zorbel +", "--exclude=zorbel + quuxer"] {
        assert_eq!(
            update(&dir, &id, &[flag]).status.code(),
            Some(4),
            "{flag:?}"
        );
    }
    assert_eq!(
        triggers_json(&dir)[0]["triggers"],
        serde_json::json!(["frob"])
    );
}

#[test]
fn a_glued_plus_warns_with_the_and_spelling_and_cpp_is_unchanged() {
    let dir = setup();
    let out = mx(
        &dir,
        &["kv", "push", "facts", "x", "--trigger", "study +blarnia"],
    );
    assert!(out.status.success(), "a warning must not block the write");
    assert!(
        stderr(&out).contains("is a phrase, not an AND -- for AND write \"study + blarnia\""),
        "{}",
        stderr(&out)
    );

    let out = mx(&dir, &["kv", "push", "facts", "x", "--trigger", "c++"]);
    assert_eq!(
        stderr(&out),
        "warning: trigger \"c++\" matches as \"c\" -- it will fire on any bare \"c\"\n"
    );
}

#[test]
fn exclude_is_set_replaced_and_cleared() {
    let dir = setup();
    let id = push_door(
        &dir,
        "the zorbel",
        &[
            "--trigger",
            "zorbel",
            "--exclude",
            "Vintage Zorbel",
            "--exclude",
            "glarp*",
        ],
    );
    let v = triggers_json(&dir);
    assert_eq!(
        v[0]["exclude"],
        serde_json::json!(["vintage zorbel", "glarp*"])
    );
    let text = ok(&mx(&dir, &["kv", "triggers"]), "kv triggers");
    assert!(
        text.contains("\n      exclude: [vintage zorbel, glarp*]\n"),
        "{text}"
    );

    // `--exclude` alone is a complete update and replaces the whole list.
    ok(&update(&dir, &id, &["--exclude", "quuxer"]), "update");
    assert_eq!(
        triggers_json(&dir)[0]["exclude"],
        serde_json::json!(["quuxer"])
    );

    ok(&update(&dir, &id, &["--exclude", ""]), "clear");
    let v = triggers_json(&dir);
    assert!(v[0].get("exclude").is_none(), "{v}");
    assert_eq!(v[0]["triggers"], serde_json::json!(["zorbel"]));
}

#[test]
fn an_exclusion_without_triggers_warns() {
    let dir = setup();
    let out = mx(&dir, &["kv", "push", "facts", "x", "--exclude", "glarnish"]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("exclusions but no triggers"),
        "{}",
        stderr(&out)
    );

    let id = push_door(&dir, "y", &["--trigger", "zorbel", "--exclude", "glarnish"]);
    let out = update(&dir, &id, &["--trigger", ""]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("exclusions but no triggers"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn check_explain_names_the_exclusion_that_held_a_door() {
    let dir = setup();
    let id = push_door(
        &dir,
        "zorbel fact",
        &["--trigger", "zorbel + quuxer*", "--exclude", "vintage"],
    );
    let check = |msg: &str, json: bool| {
        let mut args = vec!["doors", "check", msg, "--session", "s1", "--explain"];
        if json {
            args.push("--json");
        }
        ok(&mx(&dir, &args), "check --explain")
    };

    let text = check("a vintage zorbel, quuxering", false);
    assert!(
        text.contains(&format!(
            "zorbel + quuxer*  facts/kv-{id}  HELD  seg 1  place -  pool -  excluded by \"vintage\"\n"
        )),
        "{text}"
    );
    let v: serde_json::Value =
        serde_json::from_str(&check("a vintage zorbel, quuxering", true)).unwrap();
    assert_eq!(v["doors"][0]["status"], "HELD");
    assert_eq!(v["doors"][0]["excluded_by"], "vintage");
    assert_eq!(v["fired"], serde_json::json!([]));

    let text = check("quuxering zorbel", false);
    assert!(
        text.contains(&format!(
            "zorbel + quuxer*  facts/kv-{id}  FIRED  seg 1  place house  pool house\n"
        )),
        "{text}"
    );
    let v: serde_json::Value = serde_json::from_str(&check("quuxering zorbel", true)).unwrap();
    assert!(v["doors"][0].get("excluded_by").is_none());
    assert_eq!(v["fired"][0]["trigger"], "zorbel + quuxer*");
}
