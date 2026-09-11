//! CLI-level integration tests for write-boundary dedup (W447).
//!
//! Root cause: mx stores `content_hash` but never enforced it, so
//! regenerated/recased duplicates (same meaning, different case/punctuation)
//! landed as separate rows. These tests drive the REAL `mx` binary against an
//! isolated, real SurrealDB (`MX_HOME` per test), covering the two write
//! funnels flagged in review: `add_one` (single add +
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
/// ambient live-DB credentials from the development environment) -- force
/// embedded/file-backed mode under the per-test `MX_HOME` so these tests
/// never touch a shared live database.
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

/// Parse `mx memory list --json` stdout (a bare JSON array of entries) into
/// its elements, so a test can assert on the actual row count in the store
/// rather than trusting only the CLI's human-readable summary line.
fn extract_json_array(text: &str) -> Vec<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text.trim())
        .unwrap_or_else(|e| panic!("failed to parse JSON array ({e}): {text:?}"))
        .as_array()
        .unwrap_or_else(|| panic!("expected a JSON array: {text:?}"))
        .clone()
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

    // Test-authority fix (fix-round review, finding 3): the CLI's
    // self-reported `"skipped": true` is not proof nothing was written --
    // ask the store directly. mystery-meat proved this gap live: a patched
    // `add_one` that still fell through to a second `upsert_knowledge` after
    // a detected duplicate kept every `dedup_gate_tests` unit test green and
    // still printed a clean `"skipped": true`, while `mx memory list` showed
    // two persisted rows. This assertion is the one thing in the suite that
    // would have caught that.
    let list = mx(&env, &["memory", "list", "--category", "insight", "--json"]);
    assert!(list.status.success(), "list failed: {}", stderr(&list));
    let entries = extract_json_array(&stdout(&list));
    assert_eq!(
        entries.len(),
        1,
        "exactly one entry should exist after a recased re-add via the standard path: {:?}",
        entries
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
        // Fix-round review, json-mode double-signal nuance: pre-fix, the
        // plain-mode stderr note fired UNCONDITIONALLY, so --json mode got
        // both the stderr note and the json field, contradicting the docs'
        // mode-exclusive phrasing ("a bypass signal ... in --json mode, and
        // a stderr note in plain mode"). In --json mode only the json field
        // should appear.
        assert!(
            !stderr(&out).contains("dedup bypassed"),
            "--json mode must not ALSO print the plain-mode stderr bypass note: {}",
            stderr(&out)
        );
    }
}

#[test]
fn session_id_none_plain_mode_prints_bypass_note_on_stderr() {
    let env = setup();

    let out = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "No Session Note Plain",
            "--content",
            "Written without a session id, plain mode.",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("dedup bypassed"),
        "plain mode must still print the stderr bypass note: {}",
        stderr(&out)
    );
}

#[test]
fn skip_json_payload_carries_the_duplicate_id_under_the_id_key() {
    // Fix-round review, minor finding: every success payload has an `id`
    // key; the skip payload had none, so `jq -r .id` silently read null on a
    // skip. The skip payload's `id` must equal `duplicate_of`.
    let env = setup();

    let first = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Has An Id",
            "--content",
            "Content for the id-key regression test.",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(first.status.success());
    let first_id = extract_json(&stdout(&first))["id"]
        .as_str()
        .unwrap()
        .to_string();

    let second = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "has an id",
            "--content",
            "content for the id-key regression test",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_str(&stdout(&second)).unwrap();
    assert_eq!(
        second_json["id"],
        serde_json::json!(first_id),
        "skip payload's 'id' key must carry the duplicate's id, not be absent: {second_json:?}"
    );
    assert_eq!(second_json["duplicate_of"], serde_json::json!(first_id));
}

// =========================================================================
// Fact-type add-batch inline path (the funnel earlier review missed entirely --
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
    // Anchored to the full summary line, not a bare `contains("1 added")`
    // (fix-round review, hygiene finding: that substring also matches "21
    // added" at larger magnitudes -- harmless today, fragile if this suite
    // is ever reused at scale).
    assert!(
        first_out
            .contains("Batch complete: 1 added, 1 already saved (no action needed), 0 failed."),
        "first run must add exactly one entry and skip exactly one: {first_out}"
    );

    // Test-authority fix (fix-round review, finding 3): confirm the actual
    // row count, not just the self-reported summary line.
    let list = mx(&env, &["memory", "list", "--category", "insight", "--json"]);
    assert!(list.status.success(), "list failed: {}", stderr(&list));
    assert_eq!(
        extract_json_array(&stdout(&list)).len(),
        1,
        "exactly one entry should exist after the first batch run"
    );

    // Re-running the SAME batch: both lines now dedup against the DB ->
    // zero new entries, proving the fact-type funnel is gated (not just
    // add_one).
    let second = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(second.status.success());
    let second_out = stdout(&second);
    assert!(
        second_out
            .contains("Batch complete: 0 added, 2 already saved (no action needed), 0 failed."),
        "re-running the identical batch must add zero new entries and skip both lines: {second_out}"
    );

    let list_again = mx(&env, &["memory", "list", "--category", "insight", "--json"]);
    assert!(list_again.status.success());
    assert_eq!(
        extract_json_array(&stdout(&list_again)).len(),
        1,
        "row count must still be exactly one after the identical batch is re-run: {:?}",
        stdout(&list_again)
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
    // Anchored to the full summary line (fix-round review, hygiene finding),
    // not loose `contains("1 added")` substrings.
    assert!(
        text.contains("Batch complete: 1 added, 1 already saved (no action needed), 0 failed."),
        "expected exactly one add and one skip: {text}"
    );

    // Test-authority fix (fix-round review, finding 3): confirm the actual
    // row count via the store, not just the self-reported summary line.
    let list = mx(&env, &["memory", "list", "--category", "insight", "--json"]);
    assert!(list.status.success(), "list failed: {}", stderr(&list));
    assert_eq!(
        extract_json_array(&stdout(&list)).len(),
        1,
        "exactly one entry should exist after a recased duplicate via the standard batch path"
    );
}

// =========================================================================
// Fact-type single-add path (`mx memory add --type <fact_type>`) -- a THIRD
// new-entry write funnel earlier review passes never enumerated. It
// builds a KnowledgeEntry inline and calls `db.upsert_knowledge` directly,
// never touching `add_one` and never (until this fix) consulting the shared
// DedupIndex. Requires `--session`: `ensure_group`/`check` both bypass on a
// `None` session by design (W447 rulings #6), so this suite must always pass
// `--session` or it would green-pass without ever exercising the gate.
// =========================================================================

#[test]
fn single_add_fact_type_path_recased_duplicate_is_skipped_exactly_one_entry() {
    let env = setup();

    let first = mx(
        &env,
        &[
            "memory",
            "add",
            "--type",
            "insight",
            "--content",
            "Ship it, and move on.",
            "--session",
            "sess-1",
            "--no-embed",
        ],
    );
    assert!(first.status.success());
    assert!(
        stdout(&first).contains("Added fact:"),
        "first add must write through: {}",
        stdout(&first)
    );

    // Recased/repunctuated re-add of the same content, same session -> must
    // be skipped, not written as a second entry.
    let second = mx(
        &env,
        &[
            "memory",
            "add",
            "--type",
            "insight",
            "--content",
            "ship it and move on",
            "--session",
            "sess-1",
            "--no-embed",
        ],
    );
    assert!(
        second.status.success(),
        "a duplicate-skip must exit 0: {}",
        stderr(&second)
    );
    let second_out = stdout(&second);
    assert!(
        second_out.contains("Already saved:"),
        "recased duplicate must be reported as an affirmative skip, not a second write: {second_out}"
    );
    assert!(
        !second_out.contains("Added fact:"),
        "recased duplicate must NOT write a second entry: {second_out}"
    );

    // Confirm exactly one entry actually landed in the store: list the
    // session's facts and count.
    let list = mx(&env, &["memory", "list", "--category", "insight", "--json"]);
    assert!(list.status.success(), "list failed: {}", stderr(&list));
    let entries = extract_json_array(&stdout(&list));
    assert_eq!(
        entries.len(),
        1,
        "exactly one entry should exist after a recased re-add via the single-add fact-type path: {:?}",
        entries
    );
}

#[test]
fn single_add_fact_type_path_allow_duplicate_forces_the_write_through() {
    let env = setup();

    // Note: same body -> same `fact_title` -> same `generate_id` output for
    // this fact-routing path, so a same-content re-add overwrites to one row
    // via `generate_id` regardless of the dedup gate (documented W447 caveat,
    // mirrors `allow_duplicate_forces_the_second_write_through` above for the
    // standard path) -- this test asserts `--allow-duplicate` bypasses the
    // skip (both writes go through, neither says "Already saved"), not a
    // phantom second row.
    for _ in 0..2 {
        let out = mx(
            &env,
            &[
                "memory",
                "add",
                "--type",
                "insight",
                "--content",
                "Ship it, and move on.",
                "--session",
                "sess-1",
                "--no-embed",
                "--allow-duplicate",
            ],
        );
        assert!(out.status.success());
        let text = stdout(&out);
        assert!(
            text.contains("Added fact:"),
            "--allow-duplicate must force every write through: {text}"
        );
        assert!(
            !text.contains("Already saved:"),
            "--allow-duplicate must bypass the dedup skip entirely: {text}"
        );
    }
}

// =========================================================================
// Bypass-signal consistency across all four write funnels (fix-round review,
// tail finding: "bypass is never silent" held on only the standard `add_one`
// caller path before this fix -- the other three emitted nothing on a
// session-less write, contrary to the documented guarantee).
// =========================================================================

#[test]
fn single_add_fact_type_path_no_session_emits_bypass_note_on_stderr() {
    let env = setup();

    let out = mx(
        &env,
        &[
            "memory",
            "add",
            "--type",
            "insight",
            "--content",
            "No session here.",
            "--no-embed",
        ],
    );
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("dedup bypassed"),
        "the single-add fact-type path must surface the bypass note when --session is omitted: {}",
        stderr(&out)
    );
}

#[test]
fn batch_fact_type_path_no_session_emits_bypass_note_on_stderr() {
    let env = setup();

    let batch = "{\"type\": \"insight\", \"content\": \"No session on this line.\"}\n";
    let out = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("dedup bypassed"),
        "the batch fact-type path must surface the bypass note when 'session' is absent: {}",
        stderr(&out)
    );
}

#[test]
fn standard_batch_path_no_session_id_emits_bypass_note_on_stderr() {
    let env = setup();

    let batch = "{\"category\": \"insight\", \"title\": \"No Session\", \"content\": \"No session_id on this line.\"}\n";
    let out = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], batch);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("dedup bypassed"),
        "the standard batch path must surface the bypass note when 'session_id' is absent: {}",
        stderr(&out)
    );
}

// =========================================================================
// Claimed-owner existence oracle (fix-round review, minor finding,
// author-disclosed and accepted): a `duplicate_of` hit confirms content
// exists under a CLAIMED owner, before any authz that would reject a forged
// write. This is accepted behavior, not a bug -- this test PINS it so a
// future change can't silently alter it without a test failure forcing the
// question back into view. Extended per the review to the batch per-line
// owner vector, the sharper form (one batch call can probe many
// owner+content combinations).
// =========================================================================

#[test]
fn batch_per_line_owner_claim_confirms_existence_of_matching_private_content() {
    let env = setup();

    // "victim" writes a private entry. The schema's `owner` field carries no
    // PERMISSIONS clause (confirmed: `schema/surrealdb-schema.surql` defines
    // it as a plain `option<string>`), so this write is not itself gated on
    // the acting agent matching the claimed owner -- the same is true of the
    // probe below, which is the point of the disclosure.
    let victim_write = mx(
        &env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Victim Secret",
            "--content",
            "Victim's private content.",
            "--owner",
            "victim",
            "--private",
            "--session-id",
            "sess-1",
            "--json",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(victim_write.status.success());
    let victim_id = extract_json(&stdout(&victim_write))["id"]
        .as_str()
        .unwrap()
        .to_string();

    // "attacker" (a different source_agent) probes the same owner+content
    // via a single add-batch line -- never having written anything as
    // "victim" themselves.
    let probe = "{\"category\": \"insight\", \"title\": \"Victim Secret\", \"content\": \"Victim's private content.\", \"private\": true, \"owner\": \"victim\", \"session_id\": \"sess-1\", \"source_agent\": \"attacker\"}\n";
    let out = mx_stdin(&env, &["memory", "add-batch", "--no-embed"], probe);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains(&format!("Already saved: {}", victim_id)),
        "a batch line claiming owner=victim with matching content confirms the private \
         entry's existence via the skip -- accepted per PR #402's Honest Disclosures, \
         extended to the batch per-line owner vector: {text}"
    );
}
