//! CLI-level integration test for the Issue #400 "hidden private entries"
//! hint on `mx memory list` / `mx memory search`.
//!
//! This drives the real binary (`CARGO_BIN_EXE_mx`) against an isolated
//! SurrealDB (`MX_SURREAL_ROOT`), so it never touches the developer's real
//! store. Its whole purpose is the STDOUT/STDERR invariance (finding S2): the
//! hint must reach STDERR only — never stdout, never `--json` — so a caller
//! piping stdout into another program sees byte-identical output whether or not
//! the hint fires.
//!
//! Design notes:
//!   - Everything runs inside ONE `#[serial]` test against ONE seeded store.
//!     Spinning up an embedded SurrealDB applies the full schema (DEFINE INDEX +
//!     default-category seeding); doing that from several test processes at once
//!     races SurrealDB's optimistic concurrency ("read or write conflict … can
//!     be retried"). One store + `#[serial]` keeps the fresh-DB init off the hot
//!     parallel path and keeps the wall-clock cost to a single init.
//!   - The `--semantic` suppression (finding W1) is pinned deterministically at
//!     the unit layer (`hidden_private_hint_tests::hint_absent_under_semantic_mode`
//!     in src/helpers.rs) rather than here: the semantic path constructs a
//!     `TractProvider`, which loads/downloads an ONNX embedding model — a
//!     network- and model-cache-dependent side effect with no place in a
//!     hermetic CLI test. That pure-function test proves the flag alone
//!     suppresses the hint, which is exactly the contract W1 asked for.

use serial_test::serial;
use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Run `mx` as `agent-a` against an isolated surreal root in `dir`.
///
/// `common::isolate` forces embedded mode so an ambient
/// `MX_SURREAL_MODE=network` in the developer's shell cannot silently redirect
/// the binary at a shared network DB — that masking is exactly what let a bogus
/// (non-seeded) category pass locally while CI, running embedded against the
/// isolated root, went red.
fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "agent-a")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn mx");
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to wait on mx")
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const HINT_MARK: &str = "private entry of yours";
const HINT_FLAG: &str = "--include-private";

#[test]
#[serial]
fn hint_reaches_stderr_only_and_never_stdout() {
    let dir = TempDir::new().unwrap();

    // `insight` is one of the categories seeded on schema application
    // (schema/surrealdb-schema.surql: bloom, decision, gotcha, insight, pattern,
    // reference, session, technique — validated at handlers/memory.rs). No
    // `categories add` is needed. NOTE: do NOT use the Wonka CLAUDE.md taxonomy
    // (discovery, recipe, method, ...) here — those are NOT mx categories and the
    // add would exit non-zero against a genuinely isolated store. Seed a single
    // OWNED-PRIVATE entry for agent-a that the public-only default hides.
    let add = mx(
        &dir,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "unique searchable widget",
            "--content",
            "unique searchable widget body",
            "--private",
        ],
    );
    assert!(
        add.status.success(),
        "private add must succeed; stderr: {}",
        stderr_of(&add)
    );

    // 1) Default `list`: the owned-private entry is hidden from the main output,
    //    so the hint nudges on STDERR and must NOT appear on STDOUT.
    let out = mx(&dir, &["memory", "list"]);
    assert!(
        out.status.success(),
        "list must succeed; stderr: {}",
        stderr_of(&out)
    );
    let (so, se) = (stdout_of(&out), stderr_of(&out));
    assert!(
        se.contains(HINT_MARK) && se.contains(HINT_FLAG),
        "list hint must appear on STDERR; stderr was: {se:?}"
    );
    assert!(
        !so.contains(HINT_MARK) && !so.contains(HINT_FLAG),
        "list hint must NEVER touch STDOUT; stdout was: {so:?}"
    );

    // 2) Text `search` matching the entry: same invariance.
    let out = mx(&dir, &["memory", "search", "widget"]);
    assert!(
        out.status.success(),
        "search must succeed; stderr: {}",
        stderr_of(&out)
    );
    let (so, se) = (stdout_of(&out), stderr_of(&out));
    assert!(
        se.contains(HINT_MARK) && se.contains(HINT_FLAG),
        "search hint must appear on STDERR; stderr was: {se:?}"
    );
    assert!(
        !so.contains(HINT_MARK) && !so.contains(HINT_FLAG),
        "search hint must NEVER touch STDOUT; stdout was: {so:?}"
    );

    // 3) `list --json`: stdout must stay valid, hint-free JSON (a consumer piping
    //    stdout must not choke on a stray note line).
    let out = mx(&dir, &["memory", "list", "--json"]);
    assert!(
        out.status.success(),
        "list --json must succeed; stderr: {}",
        stderr_of(&out)
    );
    let so = stdout_of(&out);
    assert!(
        !so.contains(HINT_MARK) && !so.contains(HINT_FLAG),
        "the hint must not appear in --json stdout; stdout was: {so:?}"
    );
    serde_json::from_str::<serde_json::Value>(so.trim())
        .expect("list --json stdout must be valid JSON even when the hint fires on stderr");

    // 4) `--include-private`: the entry is already shown, so the hint must NOT
    //    fire on either stream (it would be redundant and misleading).
    let out = mx(&dir, &["memory", "list", "--include-private"]);
    assert!(out.status.success(), "list --include-private must succeed");
    let (so, se) = (stdout_of(&out), stderr_of(&out));
    assert!(
        so.contains("unique searchable widget"),
        "the private entry must be visible with --include-private; stdout: {so:?}"
    );
    assert!(
        !se.contains(HINT_MARK) && !so.contains(HINT_MARK),
        "no hint when --include-private already reveals the entry; stdout: {so:?} stderr: {se:?}"
    );
}
