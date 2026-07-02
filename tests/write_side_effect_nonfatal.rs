//! Integration tests for the post-write side-effect non-fatal fix (W446).
//!
//! Root cause: durable write commands (Add/Update/Edit/Append/Prepend/Restore)
//! commit the entry, then run `auto_embed`/`auto_anchor` as best-effort
//! side-effects. Before this fix those side effects were chained with `?`,
//! so a transient embed/anchor failure propagated all the way to `main()`
//! and produced a non-zero exit *after the write had already landed* --
//! callers would see "failure", retry, and duplicate the entry.
//!
//! These tests force `auto_embed` to fail deterministically (no network
//! flakiness assumed beyond reachability of huggingface.co, which the rest
//! of this suite already depends on for real embedding) and assert:
//!   - the process still exits 0,
//!   - a warning naming the entry as durable is printed to stderr,
//!   - the write is actually visible afterward.
//!
//! A genuine write failure (the write itself never lands) must still exit
//! non-zero -- that path is untouched by this fix and is asserted here too.

use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Each test spins up a full SurrealDB + embedding engine under its own temp
/// MX_HOME. Running them concurrently exhausts the (shared) model cache and DB
/// resources, so serialize the heavyweight binary invocations through one lock.
/// Mirrors the harness style in `tests/triggers_cli.rs`.
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

fn mx(env: &Env, args: &[&str]) -> std::process::Output {
    Command::new(MX)
        .args(args)
        .env("MX_HOME", env.dir.path())
        .env("MX_CURRENT_AGENT", "test")
        .output()
        .expect("failed to run mx")
}

/// Runs an `mx` command with the on-write embed step forced to fail.
///
/// `MX_ISOLATE_MODELS=1` points the hf-hub model cache at
/// `$MX_HOME/memory/embed` (see `src/paths.rs::model_cache_dir`). Pre-creating
/// that path as a plain *file* (not a directory) makes hf-hub's
/// `create_dir_all(blob_path.parent())` fail as soon as it tries to stage the
/// downloaded model blob -- deterministic and independent of whether the
/// model happens to already be warm in the shared cache. The failure occurs
/// after `metadata()`'s network round-trip, so this still requires
/// huggingface.co to be reachable, same as every other embedding test in
/// this suite.
fn mx_with_broken_model_cache(env: &Env, args: &[&str]) -> std::process::Output {
    let cache_block = env.dir.path().join("memory").join("embed");
    std::fs::create_dir_all(cache_block.parent().unwrap()).unwrap();
    std::fs::write(&cache_block, b"blocking file, not a directory").unwrap();

    Command::new(MX)
        .args(args)
        .env("MX_HOME", env.dir.path())
        .env("MX_CURRENT_AGENT", "test")
        .env("MX_ISOLATE_MODELS", "1")
        .output()
        .expect("failed to run mx")
}

/// Add an entry with embed/anchor skipped, so entry creation itself never
/// touches the model cache. Returns its `kn-` id.
///
/// Deliberately omits `--json`: `add_one`'s `--no-embed`/`--no-auto-anchor`
/// branches `println!` a "(... skipped)" notice to stdout ahead of whatever
/// the caller prints, which corrupts `--json`'s stdout-is-JSON contract.
/// That's a pre-existing bug independent of this test file, so this helper
/// works around it by parsing the plain-text "Added entry: <id>" line
/// instead of asking for JSON.
fn add_plain_no_embed(env: &Env) -> String {
    let out = mx(
        env,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Nonfatal Side-Effect Test",
            "--content",
            "original body",
            "--no-embed",
            "--no-auto-anchor",
        ],
    );
    assert!(
        out.status.success(),
        "setup add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("Added entry: "))
        .map(|id| id.trim().to_string())
        .unwrap_or_else(|| panic!("expected an 'Added entry: <id>' line, got: {stdout}"))
}

fn show_body(env: &Env, id: &str) -> String {
    let out = mx(env, &["memory", "show", id, "--json"]);
    assert!(
        out.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["body"].as_str().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// Post-write side-effect failure is non-fatal
// ---------------------------------------------------------------------------

#[test]
fn append_embed_failure_is_non_fatal_and_content_persists() {
    let dir = setup();
    let id = add_plain_no_embed(&dir);

    let out = mx_with_broken_model_cache(
        &dir,
        &["memory", "append", &id, "--content", "appended text"],
    );

    assert!(
        out.status.success(),
        "append must exit 0 even when the post-write embed fails: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("post-write embed failed") && stderr.contains("entry durable"),
        "expected a non-fatal embed warning naming the entry as durable: {stderr}"
    );

    // The append landed despite the embed step failing.
    assert!(
        show_body(&dir, &id).contains("appended text"),
        "appended content should be durable even though embed failed"
    );
}

#[test]
fn add_embed_failure_is_non_fatal_and_entry_persists() {
    let dir = setup();

    let out = mx_with_broken_model_cache(
        &dir,
        &[
            "memory",
            "add",
            "--category",
            "insight",
            "--title",
            "Add Nonfatal Embed Test",
            "--content",
            "brand new entry",
            "--json",
        ],
    );

    assert!(
        out.status.success(),
        "add must exit 0 even when the post-write embed fails: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("post-write embed failed") && stderr.contains("entry durable"),
        "expected a non-fatal embed warning naming the entry as durable: {stderr}"
    );

    // The entry actually landed despite the embed step failing. `add --json`
    // still reports the id on the (now non-fatal) embed-failure path.
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("add --json output should still parse on the non-fatal embed path");
    let id = v["id"].as_str().expect("id present").to_string();
    assert_eq!(show_body(&dir, &id), "brand new entry");
}

#[test]
fn add_batch_embed_failure_is_non_fatal_and_entries_persist() {
    let dir = setup();

    let batch_file = dir.dir.path().join("batch.jsonl");
    std::fs::write(
        &batch_file,
        concat!(
            r#"{"category":"insight","title":"Batch Nonfatal 1","content":"first entry","source_agent":"test"}"#,
            "\n",
            r#"{"category":"insight","title":"Batch Nonfatal 2","content":"second entry","source_agent":"test"}"#,
            "\n",
        ),
    )
    .unwrap();

    let out = mx_with_broken_model_cache(
        &dir,
        &[
            "memory",
            "add-batch",
            "--file",
            batch_file.to_str().unwrap(),
        ],
    );

    assert!(
        out.status.success(),
        "add-batch must exit 0 even when the hoisted post-write embed pass fails: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("post-write batch embed failed") && stderr.contains("entries durable"),
        "expected a non-fatal batch embed warning naming the entries as durable: {stderr}"
    );

    // Both entries landed despite the hoisted embed pass failing entirely
    // (the model cold-load itself is what's broken here, so no entry embeds).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let ids: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.split_whitespace().find(|w| w.starts_with("kn-")))
        .map(|w| w.trim_end_matches(':').trim_start_matches('[').trim())
        .collect();
    assert_eq!(
        ids.len(),
        2,
        "expected both batch entries to report an id in stdout, got: {stdout}"
    );
    for id in ids {
        let body = show_body(&dir, id);
        assert!(
            body == "first entry" || body == "second entry",
            "batch entry {id} should be durable with its content, got body: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// A genuine write failure must still exit non-zero
// ---------------------------------------------------------------------------

#[test]
fn append_to_missing_entry_still_exits_nonzero() {
    let dir = setup();

    let out = mx(
        &dir,
        &["memory", "append", "kn-does-not-exist", "--content", "x"],
    );

    assert!(
        !out.status.success(),
        "appending to a nonexistent entry must still fail -- the write itself never lands"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("not found"),
        "expected a not-found error, got: {stderr}"
    );
}

#[test]
fn add_with_invalid_category_still_exits_nonzero() {
    let dir = setup();

    let out = mx(
        &dir,
        &[
            "memory",
            "add",
            "--category",
            "not-a-real-category",
            "--title",
            "Should Not Land",
            "--content",
            "body",
        ],
    );

    assert!(
        !out.status.success(),
        "an invalid category must still fail before any write is attempted"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Invalid category"),
        "expected the category validation error, got: {stderr}"
    );
}
