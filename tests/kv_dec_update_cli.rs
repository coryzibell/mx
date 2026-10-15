//! Integration tests for `mx kv dec` refusing to fake a decrement (#476, #457)
//! and for `mx kv update --memory` (#480).
//!
//! Harness mirrors `tests/kv_set_cli.rs`: the built binary against an isolated
//! MX_HOME, so nothing touches the live store. Every refusal is checked against
//! the raw data-file bytes, not just the read-back value.

use std::process::Command;

use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.warmth]
type = "counter"
min = 0

[keys.tally]
type = "counter"

[keys.notes]
type = "history"
"#;

fn setup() -> TempDir {
    let dir = TempDir::new().unwrap();
    write_schema(&dir, SCHEMA);
    dir
}

fn write_schema(dir: &TempDir, schema: &str) {
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), schema).unwrap();
}

fn run(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(MX);
    common::isolate(&mut c, dir.path());
    c.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .output()
        .expect("failed to run mx")
}

fn run_ok(dir: &TempDir, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        out.status.success(),
        "{:?}: exit {:?}, stderr {:?}",
        args,
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn counter(dir: &TempDir, key: &str) -> i64 {
    run_ok(dir, &["kv", "get", key])
        .parse()
        .expect("counter should print an integer")
}

fn data_bytes(dir: &TempDir) -> Vec<u8> {
    std::fs::read(dir.path().join("kv").join("data").join("test.json"))
        .expect("data file should exist after a write")
}

/// Run a command that must be refused, then prove the data file was not touched.
fn assert_refused(dir: &TempDir, args: &[&str], code: i32, stderr_has: &[&str]) {
    let bytes_before = data_bytes(dir);

    let out = run(dir, args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(code),
        "{:?}: stdout {:?}, stderr {:?}",
        args,
        String::from_utf8_lossy(&out.stdout),
        stderr
    );
    assert!(
        out.stdout.is_empty(),
        "{:?} printed to stdout: {:?}",
        args,
        String::from_utf8_lossy(&out.stdout)
    );
    for needle in stderr_has {
        assert!(
            stderr.contains(needle),
            "{:?}: stderr missing {:?}: {}",
            args,
            needle,
            stderr
        );
    }
    assert_eq!(
        data_bytes(dir),
        bytes_before,
        "{:?} rewrote the data file",
        args
    );
}

// -- `kv dec` --

#[test]
fn dec_by_zero_is_invalid_input() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "warmth", "5"]);
    assert_refused(
        &dir,
        &["kv", "dec", "warmth", "--by", "0"],
        4,
        &[
            "must be positive",
            "got 0",
            "mx kv inc",
            "Nothing was written",
        ],
    );
    assert_eq!(counter(&dir, "warmth"), 5);
}

#[test]
fn dec_by_negative_is_invalid_input_in_both_spellings() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "warmth", "5"]);
    for args in [
        &["kv", "dec", "warmth", "--by=-3"][..],
        &["kv", "dec", "warmth", "--by", "-3"][..],
    ] {
        assert_refused(&dir, args, 4, &["must be positive", "got -3", "mx kv inc"]);
    }
    assert_eq!(counter(&dir, "warmth"), 5);
}

#[test]
fn dec_at_min_is_refused() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "warmth", "0"]);
    assert_refused(
        &dir,
        &["kv", "dec", "warmth"],
        5,
        &[
            "'warmth'",
            "is 0",
            "become 0",
            "min is 0",
            "Nothing was written",
        ],
    );
    assert_eq!(counter(&dir, "warmth"), 0);
}

#[test]
fn dec_below_a_raised_min_is_refused() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "tally", "--", "-5"]);
    write_schema(
        &dir,
        &SCHEMA.replace(
            "[keys.tally]\ntype = \"counter\"",
            "[keys.tally]\ntype = \"counter\"\nmin = 0",
        ),
    );
    assert_refused(
        &dir,
        &["kv", "dec", "tally"],
        5,
        &["'tally'", "is -5", "become 0", "min is 0"],
    );
    assert_eq!(counter(&dir, "tally"), -5);
}

#[test]
fn dec_partial_clamp_lowers_to_min() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "warmth", "5"]);
    assert_eq!(run_ok(&dir, &["kv", "dec", "warmth", "--by", "10"]), "0");
    assert_eq!(counter(&dir, "warmth"), 0);
}

#[test]
fn plain_dec_still_works() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "warmth", "5"]);
    assert_eq!(run_ok(&dir, &["kv", "dec", "warmth"]), "4");
    assert_eq!(run_ok(&dir, &["kv", "dec", "warmth", "--by", "2"]), "2");
    assert_eq!(counter(&dir, "warmth"), 2);
}

#[test]
fn inc_by_negative_with_a_space_is_still_invalid_input() {
    let dir = setup();
    run_ok(&dir, &["kv", "set", "tally", "5"]);
    assert_refused(
        &dir,
        &["kv", "inc", "tally", "--by", "-3"],
        4,
        &["must be positive", "got -3", "mx kv dec"],
    );
    assert_eq!(counter(&dir, "tally"), 5);
}

// -- `kv update --memory` (#480) --

/// Push one entry onto `notes` and return its index as the `--id` argument.
fn push_note(dir: &TempDir, value: &str) -> String {
    run_ok(dir, &["kv", "push", "notes", value]);
    let entries: serde_json::Value =
        serde_json::from_str(&run_ok(dir, &["kv", "get", "notes", "--json"])).unwrap();
    entries.as_array().unwrap().last().unwrap()["index"].to_string()
}

fn note(dir: &TempDir, id: &str) -> serde_json::Value {
    let entries: serde_json::Value =
        serde_json::from_str(&run_ok(dir, &["kv", "get", "notes", "--json"])).unwrap();
    entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["index"].as_u64() == id.parse().ok())
        .cloned()
        .unwrap_or_else(|| panic!("no note with index {}", id))
}

#[test]
fn update_memory_sets_replaces_and_clears() {
    let dir = setup();
    let id = push_note(&dir, "hello");

    run_ok(
        &dir,
        &[
            "kv",
            "update",
            "notes",
            "--id",
            &id,
            "--memory",
            "kn-1a2b3c4d",
        ],
    );
    let e = note(&dir, &id);
    assert_eq!(e["memory"], "kn-1a2b3c4d");
    assert_eq!(e["value"], "hello");

    run_ok(
        &dir,
        &["kv", "update", "notes", "--id", &id, "--memory", "kn-dup01"],
    );
    assert_eq!(note(&dir, &id)["memory"], "kn-dup01");

    run_ok(
        &dir,
        &["kv", "update", "notes", "--id", &id, "--memory", ""],
    );
    assert!(note(&dir, &id).get("memory").is_none());
}

#[test]
fn update_memory_alone_passes_the_no_op_guard() {
    let dir = setup();
    let id = push_note(&dir, "hello");

    let out = run(&dir, &["kv", "update", "notes", "--id", &id]);
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--memory"));

    run_ok(
        &dir,
        &["kv", "update", "notes", "--id", &id, "--memory", "kn-abc"],
    );
    run_ok(
        &dir,
        &[
            "kv", "update", "notes", "--id", &id, "--data", "{}", "--memory", "kn-def",
        ],
    );
    assert_eq!(note(&dir, &id)["memory"], "kn-def");
}

#[test]
fn update_bad_memory_is_invalid_input_and_writes_nothing() {
    let dir = setup();
    let id = push_note(&dir, "hello");
    run_ok(
        &dir,
        &["kv", "update", "notes", "--id", &id, "--memory", "kn-keep1"],
    );

    for bad in ["foo", "kn-", "kv-abc", "kn-a b", "kn-x/y"] {
        assert_refused(
            &dir,
            &["kv", "update", "notes", "--id", &id, "--memory", bad],
            4,
            &[&format!("{:?}", bad), "Nothing was written"],
        );
        assert_refused(
            &dir,
            &[
                "kv",
                "update",
                "notes",
                "new value",
                "--id",
                &id,
                "--memory",
                bad,
            ],
            4,
            &[&format!("{:?}", bad)],
        );
    }

    let e = note(&dir, &id);
    assert_eq!(e["value"], "hello");
    assert_eq!(e["memory"], "kn-keep1");
}

#[test]
fn set_with_id_shares_the_memory_shape_check() {
    let dir = setup();
    let id = push_note(&dir, "hello");
    run_ok(
        &dir,
        &["kv", "set", "notes", "--id", &id, "--memory", "kn-keep1"],
    );

    assert_refused(
        &dir,
        &["kv", "set", "notes", "--id", &id, "--memory", "foo"],
        4,
        &["\"foo\"", "Nothing was written"],
    );
    assert_eq!(note(&dir, &id)["memory"], "kn-keep1");
}
