//! Integration tests for `mx kv set` argument binding (Issue #386).
//!
//! `trailing_var_arg` on `KvCommands::Set` captured trailing flag-like tokens
//! as positional values, silently corrupting the store (`set k world --json`
//! stored the literal `--json` and dropped `world`, exit 0). These tests pin
//! the post-fix behavior: declared flags bind anywhere on the line, two-token
//! scalar values fail loudly, and `--` remains the escape hatch for literal
//! hyphen-leading values. Harness mirrors `tests/triggers_cli.rs` (built
//! binary against an isolated MX_HOME); no lock needed since `mx kv` touches
//! only flat files under MX_HOME.

use std::process::Command;
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.note]
type = "string"

[keys.warmth]
type = "counter"

[keys.tensor]
type = "state"
fields = ["temperature", "entropy"]
"#;

/// Fresh isolated MX_HOME with a seeded kv schema for agent `test`.
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
        .output()
        .expect("failed to run mx")
}

fn get(dir: &TempDir, key: &str) -> String {
    let out = mx(dir, &["kv", "get", key]);
    assert!(
        out.status.success(),
        "get {} failed: {}",
        key,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn set_ok(dir: &TempDir, args: &[&str]) {
    let mut full = vec!["kv", "set"];
    full.extend_from_slice(args);
    let out = mx(dir, &full);
    assert!(
        out.status.success(),
        "set {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

// -- trailing --json --

#[test]
fn trailing_bare_json_flag_errors_string_key() {
    let dir = setup();
    set_ok(&dir, &["note", "hello"]);

    // Bare trailing --json binds to the declared flag and is missing its value.
    let out = mx(&dir, &["kv", "set", "note", "world", "--json"]);
    assert!(!out.status.success());
    assert_eq!(get(&dir, "note"), "hello", "store must be unchanged");
}

#[test]
fn json_flag_with_positionals_hits_combined_guard() {
    let dir = setup();
    set_ok(&dir, &["note", "hello"]);

    let out = mx(
        &dir,
        &["kv", "set", "note", "world", "--json", "{\"a\":\"b\"}"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be combined"),
        "expected combined guard, got: {}",
        stderr
    );
    assert_eq!(get(&dir, "note"), "hello", "store must be unchanged");
}

#[test]
fn trailing_json_flag_errors_counter_key() {
    let dir = setup();
    set_ok(&dir, &["warmth", "3"]);

    let out = mx(&dir, &["kv", "set", "warmth", "5", "--json"]);
    assert!(!out.status.success());
    assert_eq!(get(&dir, "warmth"), "3", "store must be unchanged");
}

#[test]
fn trailing_json_flag_errors_state_key() {
    let dir = setup();
    set_ok(&dir, &["tensor", "temperature=0.5"]);

    let out = mx(&dir, &["kv", "set", "tensor", "entropy", "--json"]);
    assert!(!out.status.success());
    let val = get(&dir, "tensor");
    assert!(val.contains("\"temperature\": \"0.5\""));
    assert!(!val.contains("--json"), "flag must not leak into the store");
}

// -- trailing -v (global verbose) --

#[test]
fn trailing_verbose_flag_binds_globally_string_key() {
    let dir = setup();

    // Pre-fix this stored the literal "-v" and dropped "world".
    let out = mx(&dir, &["kv", "set", "note", "world", "-v"]);
    assert!(
        out.status.success(),
        "set failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(get(&dir, "note"), "world");
}

#[test]
fn trailing_verbose_flag_binds_globally_counter_key() {
    let dir = setup();

    let out = mx(&dir, &["kv", "set", "warmth", "5", "-v"]);
    assert!(
        out.status.success(),
        "set failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(get(&dir, "warmth"), "5");
}

#[test]
fn trailing_verbose_flag_binds_globally_state_key() {
    let dir = setup();

    let out = mx(&dir, &["kv", "set", "tensor", "temperature", "0.7", "-v"]);
    assert!(
        out.status.success(),
        "set failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(get(&dir, "tensor").contains("\"temperature\": \"0.7\""));
}

// -- silent first-token drop --

#[test]
fn unquoted_two_word_string_value_errors() {
    let dir = setup();
    set_ok(&dir, &["note", "hello"]);

    // Pre-fix this stored "words" and silently dropped "two".
    let out = mx(&dir, &["kv", "set", "note", "two", "words"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("quote the value"),
        "error must name the problem, got: {}",
        stderr
    );
    assert_eq!(get(&dir, "note"), "hello", "store must be unchanged");
}

#[test]
fn counter_two_positionals_error() {
    let dir = setup();
    set_ok(&dir, &["warmth", "3"]);

    // Pre-fix this stored 5 and silently dropped 1.
    let out = mx(&dir, &["kv", "set", "warmth", "1", "5"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is counter"),
        "error must name the type, got: {}",
        stderr
    );
    assert_eq!(get(&dir, "warmth"), "3", "store must be unchanged");
}

// -- escape hatch and legit flag-after-value usage --

#[test]
fn double_dash_stores_literal_flag_token() {
    let dir = setup();

    set_ok(&dir, &["note", "--", "--json"]);
    assert_eq!(get(&dir, "note"), "--json");
}

#[test]
fn memory_flag_after_value_works_on_state_key() {
    let dir = setup();

    // Pre-fix: --memory was swallowed into args → exit 4.
    set_ok(
        &dir,
        &["tensor", "temperature", "0.7", "--memory", "kn-abc123"],
    );
    assert!(get(&dir, "tensor").contains("\"temperature\": \"0.7\""));

    let data = std::fs::read_to_string(dir.path().join("kv").join("data").join("test.json"))
        .expect("data file written");
    assert!(
        data.contains("kn-abc123"),
        "memory pointer must be persisted: {}",
        data
    );
}

// -- batch key=value form (#324) unaffected --

#[test]
fn batch_key_value_form_unaffected() {
    let dir = setup();

    set_ok(&dir, &["tensor", "temperature=0.5", "entropy=0.2"]);
    let val = get(&dir, "tensor");
    assert!(val.contains("\"temperature\": \"0.5\""));
    assert!(val.contains("\"entropy\": \"0.2\""));
}
