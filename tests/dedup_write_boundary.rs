//! CLI-level integration tests for write-boundary dedup (W447).
//!
//! Root cause: mx stores `content_hash` but never enforced it, so
//! regenerated/recased duplicates (same meaning, different case/punctuation)
//! landed as separate rows. These tests drive the REAL `mx` binary against an
//! isolated, real SurrealDB (`MX_HOME` per test), covering the two write
//! funnels the round-1/round-2 panels flagged: `add_one` (single add +
//! batch-standard) and the add-batch fact-type inline path -- proving BOTH
//! are gated, not just the one that's easy to test.
//!
//! `--no-embed --no-auto-anchor` on every write: entry creation must never
//! touch the embedding model cache (network-dependent, flaky in CI). The
//! write-boundary dedup gate runs before either side effect, so this doesn't
//! change what's under test.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Serialize the heavyweight binary invocations (mirrors
/// `write_side_effect_nonfatal.rs`'s harness style).
fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct Env {
    dir: TempDir,
    _guard: MutexGuard<'static, ()>,
}

fn setup() -> Env {
    let guard = test_lock().lock().unwrap_or_else(|e| e.into_inner());
    Env {
        dir: TempDir::new().unwrap(),
        _guard: guard,
    }
}

/// Isolate the child from any ambient `MX_SURREAL_*` network config in the
/// invoking shell (this dev sandbox exports `MX_SURREAL_MODE=network` +
/// live-DB credentials for the Soren hearth) -- force embedded/file-backed
/// mode under the per-test `MX_HOME` so these tests never touch a shared
/// live database.
fn isolate(cmd: &mut Command, home: &std::path::Path) {
    cmd.env("MX_HOME", home)
        .env("MX_CURRENT_AGENT", "test-agent")
        .env("MX_SURREAL_MODE", "embedded")
        .env_remove("MX_SURREAL_URL")
        .env_remove("MX_SURREAL_NS")
        .env_remove("MX_SURREAL_DB")
        .env_remove("MX_SURREAL_USER")
        .env_remove("MX_SURREAL_AUTH_LEVEL")
        .env_remove("MX_SURREAL_ROOT");
}

fn mx(env: &Env, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    cmd.args(args);
    isolate(&mut cmd, env.dir.path());
    cmd.output().expect("failed to run mx")
}

fn mx_stdin(env: &Env, args: &[&str], stdin: &str) -> std::process::Output {
    let mut cmd = Command::new(MX);
    cmd.args(args);
    isolate(&mut cmd, env.dir.path());
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn mx");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to wait on mx")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Extract the pretty-printed JSON object from a WRITE-path `--json`
/// response. Pre-existing, out-of-scope bug (flagged separately, inherited
/// from the #399 fix): on the WRITE path, `add_one`'s `(embed skipped)` /
/// `(auto-anchor skipped)` notices print to stdout BEFORE the caller's JSON
/// payload when `--no-embed`/`--no-auto-anchor` are set, so write-path
/// `--json` stdout is not pure JSON. The dedup SKIP path is unaffected (it
/// returns before those notices) -- see `standard_path_recased_duplicate_is_skipped_exactly_one_entry`,
/// which asserts the skip path's stdout parses as JSON with no slicing.
fn extract_json(text: &str) -> serde_json::Value {
    let start = text
        .find("{\n")
        .unwrap_or_else(|| panic!("no JSON object found in stdout: {text:?}"));
    serde_json::from_str(text[start..].trim())
        .unwrap_or_else(|e| panic!("failed to parse extracted JSON ({e}): {:?}", &text[start..]))
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// =========================================================================
// Standard path (add_one), single add
// =========================================================================

#[test]
fn standard_path_recased_duplicate_is_skipped_exactly_one_entry() {
    let env = setup();

    let first = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "The External Plan",
            "--content",
            "Ship it, and move on.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(
        first.status.success(),
        "first add must succeed: {}",
        stderr(&first)
    );
    let first_json = extract_json(&stdout(&first));
    let first_id = first_json["id"].as_str().unwrap().to_string();

    // Recased + repunctuated variant, same session, same (implicit) owner.
    let second = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "the external plan",
            "--content",
            "ship it and move on",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(
        second.status.success(),
        "a duplicate-skip must still exit 0: {}",
        stderr(&second)
    );
    let second_json: serde_json::Value = serde_json::from_str(&stdout(&second)).unwrap();
    assert_eq!(second_json["skipped"], serde_json::json!(true));
    assert_eq!(
        second_json["status"],
        serde_json::json!("already_persisted")
    );
    assert_eq!(second_json["duplicate_of"], serde_json::json!(first_id));

    // No non-JSON text on stdout in json mode for the skip path.
    assert!(
        serde_json::from_str::<serde_json::Value>(stdout(&second).trim()).is_ok(),
        "skip --json stdout must be pure JSON, got: {:?}",
        stdout(&second)
    );
}

#[test]
fn evolved_re_add_different_title_and_body_produces_two_entries() {
    let env = setup();

    let first = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Plan A",
            "--content",
            "Ship it and move on.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(first.status.success());

    // Genuinely different title AND body -- must NOT be treated as a
    // duplicate (different generate_id, different dedup_hash).
    let second = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Plan B",
            "--content",
            "Hold off and reassess next week.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(second.status.success());
    let second_json = extract_json(&stdout(&second));
    assert!(
        second_json.get("skipped").is_none(),
        "a genuinely different entry must not be skipped: {:?}",
        second_json
    );
}

#[test]
fn allow_duplicate_forces_the_second_write_through() {
    let env = setup();

    let first = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Repeated Note",
            "--content",
            "Same content twice, on purpose.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(first.status.success());

    let second = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Repeated Note",
            "--content",
            "Same content twice, on purpose.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
            "--allow-duplicate",
        ],
    );
    assert!(second.status.success());
    let second_json = extract_json(&stdout(&second));
    assert!(
        second_json.get("skipped").is_none(),
        "--allow-duplicate must force the write through: {:?}",
        second_json
    );
}

#[test]
fn session_id_none_writes_through_with_no_dedup() {
    let env = setup();

    for _ in 0..2 {
        let out = mx(
            &env,
            &[
                "memory",
                "add",
                "--category",
                "insight",
                "--title",
                "No Session Note",
                "--content",
                "Written without a session id.",
                "--json",
                "--no-embed",
                "--no-auto-anchor",
            ],
        );
        assert!(out.status.success());
        let json = extract_json(&stdout(&out));
        assert!(
            json.get("skipped").is_none(),
            "session_id=None must bypass dedup entirely (write through): {:?}",
            json
        );
        assert_eq!(
            json["dedup"],
            serde_json::json!("bypassed_no_session"),
            "json payload must surface the bypass signal"
        );
    }
}

// =========================================================================
// Fact-type add-batch inline path (the funnel round-1 missed entirely --
// never touches add_one)
// =========================================================================

#[test]
fn fact_type_batch_path_idempotent_across_reruns() {
    let env = setup();

    let batch = "{\"type\": \"insight\", \"content\": \"Ship it, and move on.\", \"session\": \"sess-1\"}\n\
                 {\"type\": \"insight\", \"content\": \"ship it and move on\", \"session\": \"sess-1\"}\n";

    // First run: two lines, one recased duplicate of the other -> lands 1.
    let first = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(
        first.status.success(),
        "batch with only a duplicate-skip (no hard failures) must still exit 0: {}",
        stderr(&first)
    );
    let first_out = stdout(&first);
    assert!(
        first_out.contains("1 added"),
        "first run must add exactly one entry: {first_out}"
    );
    assert!(
        first_out.contains("1 already saved"),
        "first run must report exactly one skip: {first_out}"
    );

    // Re-running the SAME batch: both lines now dedup against the DB ->
    // zero new entries, proving the fact-type funnel is gated (not just
    // add_one).
    let second = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(second.status.success());
    let second_out = stdout(&second);
    assert!(
        second_out.contains("0 added"),
        "re-running the identical batch must add zero new entries: {second_out}"
    );
    assert!(
        second_out.contains("2 already saved"),
        "re-running the identical batch must skip both lines: {second_out}"
    );
}

#[test]
fn batch_skip_line_is_positional_and_carries_duplicate_of_id() {
    let env = setup();

    let batch = "{\"type\": \"insight\", \"content\": \"Ship it, and move on.\", \"session\": \"sess-1\"}\n\
                 {\"type\": \"insight\", \"content\": \"ship it and move on\", \"session\": \"sess-1\"}\n";
    mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    let out = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    let text = stdout(&out);

    // Positional [1] / [2] lines must both read as affirmative "Already
    // saved" -- never the bare words "skipped" or "duplicate" -- and must
    // carry a kn- id so positional mark-back can tag the right capture line.
    assert!(
        text.contains("[1] Already saved: kn-"),
        "line 1 must be a positional 'Already saved' line with a kn- id: {text}"
    );
    assert!(
        text.contains("[2] Already saved: kn-"),
        "line 2 must be a positional 'Already saved' line with a kn- id: {text}"
    );
    assert!(
        !text.to_lowercase().contains("skipped:") && !text.to_lowercase().contains("duplicate:"),
        "per-entry skip lines must never use bare 'skipped'/'duplicate' framing: {text}"
    );
}

#[test]
fn standard_batch_path_also_dedups_via_add_one() {
    let env = setup();

    // Title case differs ("Batch Note" vs "batch note") so `generate_id`
    // (title+path keyed, case-sensitive) would produce TWO DIFFERENT ids --
    // i.e. without the dedup gate this lands as two separate rows, which is
    // exactly the W447 evidence class (regenerated duplicates differ only by
    // case/punctuation). `dedup_hash` normalizes case, so the gate must
    // still catch it.
    let batch = "{\"category\": \"insight\", \"title\": \"Batch Note\", \"content\": \"Same body twice.\", \"session_id\": \"sess-1\"}\n\
                 {\"category\": \"insight\", \"title\": \"batch note\", \"content\": \"same body twice.\", \"session_id\": \"sess-1\"}\n";
    let out = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("1 added"), "expected exactly one add: {text}");
    assert!(
        text.contains("1 already saved"),
        "expected exactly one skip: {text}"
    );
    assert!(text.contains("0 failed"), "expected zero failures: {text}");
}
