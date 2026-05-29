//! CLI-level integration tests for `mx memory trigger-check` / `trigger-reset`
//! (Issue #246, PR3).
//!
//! These drive the real binary (`CARGO_BIN_EXE_mx`) with an isolated SurrealDB
//! (`MX_SURREAL_ROOT`) and an isolated fired-state file
//! (`MX_TRIGGER_FIRED_PATH`), so they never touch the developer's real memory
//! store or `/tmp/wonka-triggered-fired.json`.
//!
//! NOTE: the matcher / cap / dedup / visibility semantics are exercised
//! exhaustively at the unit + store-integration layer (src/triggers.rs,
//! src/surreal_db/tests.rs). Setting up TRIGGERED memories end-to-end through
//! the CLI requires `mx memory add --triggers`, which lands in PR2 — so these
//! CLI tests focus on the parts reachable without seeded triggers: input
//! handling (arg vs stdin, exit codes), empty-on-no-match output contracts, and
//! the fired-state file lifecycle (reset).

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Run `mx` with isolated surreal root + fired-state path. `stdin` is optional.
fn mx(dir: &TempDir, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(MX);
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test-agent")
        .env("MX_SURREAL_ROOT", dir.path().join("surreal"))
        .env("MX_TRIGGER_FIRED_PATH", dir.path().join("fired.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn mx");
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    // Close stdin so a stdin-fallback read reaches EOF instead of blocking.
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to wait on mx")
}

#[test]
fn empty_message_arg_exits_4() {
    let dir = TempDir::new().unwrap();
    // Explicit empty-string arg, AND empty stdin -> invalid input -> exit 4.
    let out = mx(&dir, &["memory", "trigger-check", ""], Some(""));
    assert_eq!(
        out.status.code(),
        Some(4),
        "empty message must exit 4; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn empty_stdin_no_arg_exits_4() {
    let dir = TempDir::new().unwrap();
    // No positional arg; empty stdin -> exit 4.
    let out = mx(&dir, &["memory", "trigger-check"], Some("   \n  "));
    assert_eq!(out.status.code(), Some(4));
}

#[test]
fn no_match_is_success_with_empty_stdout() {
    let dir = TempDir::new().unwrap();
    // Fresh empty store: nothing can match. Firing nothing is SUCCESS (exit 0)
    // and stdout must be empty (caller injects nothing).
    let out = mx(
        &dir,
        &["memory", "trigger-check", "an unremarkable message"],
        None,
    );
    assert!(
        out.status.success(),
        "no-match must be success (exit 0); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "no-match context output must be empty, got: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn no_match_json_reports_empty_fired_and_zero_deferred() {
    let dir = TempDir::new().unwrap();
    let out = mx(
        &dir,
        &["memory", "trigger-check", "nothing here", "--json"],
        None,
    );
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("trigger-check --json must emit valid JSON");
    assert_eq!(v["fired"].as_array().unwrap().len(), 0);
    assert_eq!(v["deferred_count"].as_i64().unwrap(), 0);
}

#[test]
fn message_from_stdin_when_arg_absent() {
    let dir = TempDir::new().unwrap();
    // Arg absent, message supplied via stdin. No matches in an empty store, but
    // the point is it must NOT exit 4 (stdin gave a non-empty message).
    let out = mx(
        &dir,
        &["memory", "trigger-check"],
        Some("a message from stdin"),
    );
    assert!(
        out.status.success(),
        "non-empty stdin message must be accepted (exit 0); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn dry_run_does_not_create_or_modify_fired_state() {
    let dir = TempDir::new().unwrap();
    let fired = dir.path().join("fired.json");
    // No matches, but --dry-run must never write the fired-state file regardless.
    let out = mx(&dir, &["memory", "trigger-check", "msg", "--dry-run"], None);
    assert!(out.status.success());
    assert!(
        !fired.exists(),
        "--dry-run must not create the fired-state file"
    );
}

#[test]
fn no_match_real_check_does_not_create_fired_state() {
    let dir = TempDir::new().unwrap();
    let fired = dir.path().join("fired.json");
    // A REAL (non-dry-run) trigger-check that matches nothing must do ZERO file
    // IO on the hot path: no empty fired-state file should be created/flocked.
    // This is the no-match case that the short-circuit in mark_survivors covers.
    let out = mx(
        &dir,
        &["memory", "trigger-check", "an unremarkable message"],
        None,
    );
    assert!(
        out.status.success(),
        "no-match must be success (exit 0); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !fired.exists(),
        "no-match real trigger-check must not create the fired-state file"
    );
}

#[test]
fn trigger_reset_removes_fired_state_file() {
    let dir = TempDir::new().unwrap();
    let fired = dir.path().join("fired.json");
    // Pre-seed a fired-state file as if a prior session marked some memories.
    fs::write(&fired, r#"{"fired":["kn-aaaaaaaa","kn-bbbbbbbb"]}"#).unwrap();
    assert!(fired.exists());

    let out = mx(&dir, &["memory", "trigger-reset"], None);
    assert!(
        out.status.success(),
        "trigger-reset must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !fired.exists(),
        "trigger-reset must remove the fired-state file"
    );
}

#[test]
fn trigger_reset_json_reports_path() {
    let dir = TempDir::new().unwrap();
    let out = mx(&dir, &["memory", "trigger-reset", "--json"], None);
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("trigger-reset --json must emit valid JSON");
    assert_eq!(v["reset"].as_bool(), Some(true));
    assert!(v["path"].as_str().unwrap().contains("fired.json"));
}

#[test]
fn trigger_reset_idempotent_when_no_file() {
    let dir = TempDir::new().unwrap();
    // No fired-state file exists yet; reset must still succeed.
    let out = mx(&dir, &["memory", "trigger-reset"], None);
    assert!(out.status.success());
}
