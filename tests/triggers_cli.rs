//! Integration tests for the triggers CLI surface (Issue #246, PR 2/4).
//!
//! Exercises `mx memory add/update/show` against a real temp database via the
//! built binary, mirroring the harness style in `tests/update_pressure.rs`.
//! Triggers normalize + dedupe through `crate::knowledge::normalize_trigger(s)`,
//! so these tests assert on the normalized/deduped forms.

use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Each test spins up a full SurrealDB + embedding engine under its own temp
/// MX_HOME. Running them concurrently exhausts the (shared) model cache and DB
/// resources, so serialize the heavyweight binary invocations through one lock.
fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Holds the serialization guard for the lifetime of the test alongside its
/// isolated MX_HOME.
struct Env {
    dir: TempDir,
    _guard: MutexGuard<'static, ()>,
}

/// Fresh isolated MX_HOME, serialized against other tests in this binary.
fn setup() -> Env {
    let guard = test_lock().lock().unwrap_or_else(|e| e.into_inner());
    Env {
        dir: TempDir::new().unwrap(),
        _guard: guard,
    }
}

fn mx(env: &Env, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, env.dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .output()
        .expect("failed to run mx")
}

/// Add an entry with the given `--triggers` flag value; returns its `kn-` id and
/// the normalized triggers the binary reported via `--json`.
fn add_with_triggers(dir: &Env, triggers: &str) -> (String, Vec<String>) {
    let out = mx(
        dir,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Trig Test",
            "--content",
            "body",
            "--triggers",
            triggers,
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("add --json output should parse");
    let id = v["id"].as_str().expect("id present").to_string();
    let triggers = json_triggers(&v);
    (id, triggers)
}

/// Add a plain entry with no triggers; returns its id.
fn add_plain(dir: &Env) -> String {
    let out = mx(
        dir,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Plain",
            "--content",
            "body",
            "--json",
        ],
    );
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["id"].as_str().unwrap().to_string()
}

/// Read the current triggers off an entry via `memory show --json`.
fn show_triggers(dir: &Env, id: &str) -> Vec<String> {
    let out = mx(dir, &["memory", "show", id, "--json"]);
    assert!(
        out.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("show --json output should parse");
    json_triggers(&v)
}

fn json_triggers(v: &serde_json::Value) -> Vec<String> {
    v["triggers"]
        .as_array()
        .expect("triggers array present")
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[test]
fn add_stores_triggers_normalized_and_deduped() {
    let dir = setup();
    // Mixed case, internal whitespace runs, and a duplicate ("brad").
    let (id, triggers) = add_with_triggers(&dir, "Blood   Sugar, brad, BRAD");
    assert_eq!(
        triggers,
        vec!["blood sugar".to_string(), "brad".to_string()],
        "triggers should be lowercased, whitespace-collapsed, and deduped"
    );
    // And the same normalized set is persisted (round-trips through show).
    assert_eq!(show_triggers(&dir, &id), triggers);
}

#[test]
fn add_without_triggers_is_empty() {
    let dir = setup();
    let id = add_plain(&dir);
    assert!(show_triggers(&dir, &id).is_empty());
}

#[test]
fn show_renders_triggers_line() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "Alpha, Beta");
    let out = mx(&dir, &["memory", "show", &id]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Triggers: alpha, beta"),
        "show output should render a Triggers line: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// update --triggers (replace whole set)
// ---------------------------------------------------------------------------

#[test]
fn update_triggers_replaces_full_set() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "old one, old two");
    let out = mx(
        &dir,
        &[
            "memory",
            "update",
            &id,
            "--triggers",
            "X One, x one, Foo Bar",
        ],
    );
    assert!(
        out.status.success(),
        "update failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Replaced wholesale, normalized + deduped ("X One"/"x one" collapse to one).
    assert_eq!(
        show_triggers(&dir, &id),
        vec!["x one".to_string(), "foo bar".to_string()]
    );
}

// ---------------------------------------------------------------------------
// update --add-trigger (append-if-absent)
// ---------------------------------------------------------------------------

#[test]
fn add_trigger_appends_normalized() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "alpha");
    let out = mx(
        &dir,
        &["memory", "update", &id, "--add-trigger", "Beta Gamma"],
    );
    assert!(out.status.success());
    assert_eq!(
        show_triggers(&dir, &id),
        vec!["alpha".to_string(), "beta gamma".to_string()]
    );
}

#[test]
fn add_trigger_existing_is_noop() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "alpha, beta");
    // Re-adding an existing trigger (different casing) must not duplicate it.
    let out = mx(&dir, &["memory", "update", &id, "--add-trigger", "ALPHA"]);
    assert!(out.status.success());
    assert_eq!(
        show_triggers(&dir, &id),
        vec!["alpha".to_string(), "beta".to_string()],
        "adding an existing trigger should be a no-op (no dupes)"
    );
}

// ---------------------------------------------------------------------------
// update --remove-trigger
// ---------------------------------------------------------------------------

#[test]
fn remove_trigger_removes_by_normalized_form() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "alpha, beta, gamma");
    // Removal compares normalized forms: "Beta " (trailing space) matches "beta".
    let out = mx(
        &dir,
        &["memory", "update", &id, "--remove-trigger", "Beta "],
    );
    assert!(out.status.success());
    assert_eq!(
        show_triggers(&dir, &id),
        vec!["alpha".to_string(), "gamma".to_string()]
    );
}

#[test]
fn remove_trigger_absent_is_clean_noop() {
    let dir = setup();
    let (id, _) = add_with_triggers(&dir, "alpha, beta");
    let out = mx(&dir, &["memory", "update", &id, "--remove-trigger", "nope"]);
    // Removing a non-present trigger succeeds and changes nothing.
    assert!(
        out.status.success(),
        "removing an absent trigger should be a clean success: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        show_triggers(&dir, &id),
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

// ---------------------------------------------------------------------------
// clap conflicts_with
// ---------------------------------------------------------------------------

#[test]
fn triggers_conflicts_with_add_trigger() {
    let dir = setup();
    let id = add_plain(&dir);
    let out = mx(
        &dir,
        &[
            "memory",
            "update",
            &id,
            "--triggers",
            "a",
            "--add-trigger",
            "b",
        ],
    );
    assert!(
        !out.status.success(),
        "--triggers + --add-trigger must be rejected by clap"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.to_lowercase().contains("usage"),
        "expected a clap conflict error: {stderr}"
    );
}

#[test]
fn triggers_conflicts_with_remove_trigger() {
    let dir = setup();
    let id = add_plain(&dir);
    let out = mx(
        &dir,
        &[
            "memory",
            "update",
            &id,
            "--triggers",
            "a",
            "--remove-trigger",
            "b",
        ],
    );
    assert!(
        !out.status.success(),
        "--triggers + --remove-trigger must be rejected by clap"
    );
}
