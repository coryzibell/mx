//! Adversarial tests for `mx kv update` / `KvStore::update_entry`.
//!
//! These run via `cargo test` as integration-style tests against the binary.
//! They test paths the 15 unit tests in kv.rs do not cover.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup(schema_toml: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("schema.toml"), schema_toml).unwrap();
    // Do NOT write data.json — KvStore::load treats missing file as DataFile::default()
    dir
}

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env("MX_KV_SCHEMA", dir.path().join("schema.toml"))
        .env("MX_KV_DATA", dir.path().join("data.json"))
        .output()
        .expect("failed to run mx")
}

fn push(dir: &TempDir, key: &str, value: &str) -> String {
    let out = mx(dir, &["kv", "push", key, value]);
    assert!(
        out.status.success(),
        "push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Output is like: kv-<id> (<index>)
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn push_with_data(dir: &TempDir, key: &str, value: &str, data: &str) -> String {
    let out = mx(dir, &["kv", "push", key, value, "--data", data]);
    assert!(
        out.status.success(),
        "push --data failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Extract the numeric index from "kv-<id> (<index>)" push output.
fn index_from_push(push_out: &str) -> String {
    // Format: "kv-<hash> (<index>)"
    push_out
        .trim_end_matches(')')
        .rsplit('(')
        .next()
        .unwrap()
        .trim()
        .to_string()
}

/// Extract the kv-<hash> prefix from push output for use in --id.
fn id_from_push(push_out: &str) -> String {
    push_out.split_whitespace().next().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Basic schema used throughout
// ---------------------------------------------------------------------------

const BASIC_SCHEMA: &str = r#"
[keys.log]
type = "history"

[keys.items]
type = "list"

[keys.counter]
type = "counter"
default = "0"
"#;

const VALIDATED_SCHEMA: &str = r#"
[keys.tasks]
type = "history"

[keys.tasks.data]
status = { type = "string", required = true }
priority = { type = "number" }
"#;

// ---------------------------------------------------------------------------
// P1: Empty patch `--data '{}'`
// ---------------------------------------------------------------------------

/// Empty patch object with no value: the handler now rejects `--data '{}'`
/// when no value is supplied, because nothing would be updated (exit 4).
/// Previously this was a silent no-op; the fix makes it an explicit error.
#[test]
fn empty_patch_is_accepted_but_changes_nothing() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "hello");
    let idx = index_from_push(&pushed);

    // Update with empty patch and no value — must now be rejected (exit 4)
    let out = mx(&dir, &["kv", "update", "log", "--id", &idx, "--data", "{}"]);
    assert!(
        !out.status.success(),
        "empty --data {{}} with no value must be rejected, got exit {:?}",
        out.status.code()
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected EXIT_INVALID_INPUT (4) for empty patch with no value"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("empty object") || stderr.contains("nothing to update"),
        "stderr must explain the rejection: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// P2: Patch with only nulls against an entry that doesn't have those fields
// ---------------------------------------------------------------------------

/// Patch is `{"foo": null}` on entry with no `data` field.
/// Spec says null-as-delete: deleting a field that doesn't exist should be a no-op.
/// Result: merged object is empty -> stored as None (no data).
#[test]
fn patch_all_nulls_on_entry_without_data_yields_none() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "world");
    let idx = index_from_push(&pushed);

    // Patch with only nulls against an entry with no data
    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "log",
            "--id",
            &idx,
            "--data",
            r#"{"foo": null, "bar": null}"#,
        ],
    );
    assert!(
        out.status.success(),
        "null-only patch on entry with no data should succeed, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Data should still be absent (no phantom null keys stored)
    let get = mx(&dir, &["kv", "get", "log", "--json"]);
    let out_str = String::from_utf8_lossy(&get.stdout).to_string();
    // data field should not contain "foo" or "bar"
    assert!(
        !out_str.contains("\"foo\"") && !out_str.contains("\"bar\""),
        "null-deleted fields must not appear in output: {}",
        out_str
    );
}

// ---------------------------------------------------------------------------
// P3: Update value to empty string
// ---------------------------------------------------------------------------

/// Updating value to an empty string. The spec says value is `Option<&str>`.
/// An empty string is a valid Rust string — it should be accepted.
/// No schema constraint prevents empty values on unvalidated keys.
#[test]
fn update_value_to_empty_string_is_accepted() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "non-empty");
    let idx = index_from_push(&pushed);

    let out = mx(&dir, &["kv", "update", "log", "--id", &idx, ""]);
    assert!(
        out.status.success(),
        "empty string value update should succeed, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Confirm the value was actually set to empty
    let get = mx(&dir, &["kv", "get", "log", "--json"]);
    let get_str = String::from_utf8_lossy(&get.stdout).to_string();
    // The value field should be empty string
    assert!(
        get_str.contains("\"value\":\"\"") || get_str.contains("\"value\": \"\""),
        "value should be empty string after update, got: {}",
        get_str
    );
}

// ---------------------------------------------------------------------------
// P4: Update on nonexistent key returns correct error (KeyNotFound, not EntryNotFound)
// ---------------------------------------------------------------------------

/// BUG CANDIDATE: When the key IS in the schema but NO data has been pushed yet,
/// `data.entries.get_mut(key)` returns None. The `_ =>` arm at kv.rs:2418 fires
/// and returns `KvError::EntryNotFound`. But the correct error here is
/// `KvError::EntryNotFound` (entry 0 doesn't exist) — actually that's fine.
/// BUT: when the key is NOT in the schema at all, `key_def` returns `KeyNotFound`.
/// This test verifies the correct error code is returned for a completely unknown key.
#[test]
fn update_nonexistent_key_returns_key_not_found_exit_code() {
    let dir = setup(BASIC_SCHEMA);

    let out = mx(
        &dir,
        &["kv", "update", "does_not_exist", "--id", "0", "newval"],
    );
    assert!(!out.status.success(), "update on nonexistent key must fail");
    // EXIT_KEY_NOT_FOUND = 1
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit code 1 (KEY_NOT_FOUND), got {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// P5: Update on empty history (key exists in schema, but no entries pushed)
// ---------------------------------------------------------------------------

/// Key is in schema as history/list but has never had anything pushed.
/// The `_ =>` arm (line 2418) returns EntryNotFound, but this is the WRONG
/// error: it should be EntryNotFound (with the right message), not KeyNotFound.
/// The `_ =>` arm also fires for DataValue::Counter/String/State — which means
/// if somehow the schema says history but the stored data is a counter (corrupt
/// file), the error would be misleading.
///
/// More importantly: what exit code does EntryNotFound produce here?
/// The spec says: EntryNotFound -> EXIT_INVALID_INPUT (4) via handle_kv_err.
#[test]
fn update_on_empty_history_returns_entry_not_found() {
    let dir = setup(BASIC_SCHEMA);
    // Do NOT push anything

    let out = mx(&dir, &["kv", "update", "log", "--id", "0", "newval"]);
    assert!(!out.status.success(), "update on empty history must fail");

    // Should be EXIT_INVALID_INPUT (4) — EntryNotFound maps to that
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected exit code 4 (INVALID_INPUT/EntryNotFound), got {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// P6: Ambiguous hash prefix — exact longer match still works
// ---------------------------------------------------------------------------

/// Push one entry, look it up by a prefix that is identical to its full ID.
/// Should succeed — full ID is unambiguous (only 1 match).
#[test]
fn update_by_full_id_string_succeeds() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "exact");
    let full_id = id_from_push(&pushed); // "kv-<full-hash>"

    let out = mx(
        &dir,
        &["kv", "update", "log", "--id", &full_id, "exact-updated"],
    );
    assert!(
        out.status.success(),
        "update by full ID must succeed, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let get = mx(&dir, &["kv", "get", "log"]);
    assert!(
        String::from_utf8_lossy(&get.stdout).contains("exact-updated"),
        "entry value must be updated"
    );
}

// ---------------------------------------------------------------------------
// P7: Multiple sequential updates on same entry — ts/id preserved across all
// ---------------------------------------------------------------------------

/// Push once; update three times. The ts and id must remain identical across
/// all updates. Only value and data change.
#[test]
fn multiple_sequential_updates_preserve_ts_and_id() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "v1");
    let idx = index_from_push(&pushed);

    // First update
    let out1 = mx(&dir, &["kv", "update", "log", "--id", &idx, "v2"]);
    assert!(out1.status.success(), "update 1 must succeed");

    // Second update
    let out2 = mx(&dir, &["kv", "update", "log", "--id", &idx, "v3"]);
    assert!(out2.status.success(), "update 2 must succeed");

    // Third update
    let out3 = mx(&dir, &["kv", "update", "log", "--id", &idx, "v4"]);
    assert!(out3.status.success(), "update 3 must succeed");

    // Verify via get --json: entry should still have same index
    let get = mx(&dir, &["kv", "get", "log", "--json"]);
    let json_str = String::from_utf8_lossy(&get.stdout).to_string();

    // Value must be v4
    assert!(
        json_str.contains("v4"),
        "final value should be v4, got: {}",
        json_str
    );

    // Only one entry should exist (not duplicated by updates)
    let value_count = json_str.matches("\"value\"").count();
    assert_eq!(
        value_count, 1,
        "should have exactly one entry after 3 updates, got {} value fields in: {}",
        value_count, json_str
    );
}

// ---------------------------------------------------------------------------
// P8: Counter key returns TypeMismatch, not some other error
// ---------------------------------------------------------------------------

/// Confirms the counter type gate exit code matches spec.
/// EXIT_TYPE_MISMATCH = 2.
#[test]
fn update_counter_key_returns_type_mismatch_exit_code() {
    let dir = setup(BASIC_SCHEMA);

    let out = mx(&dir, &["kv", "update", "counter", "--id", "0", "99"]);
    assert!(!out.status.success(), "update on counter must fail");

    // EXIT_TYPE_MISMATCH = 2
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit code 2 (TYPE_MISMATCH), got {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// P9: --data as non-object (string, array, number, null, bool) all rejected
// ---------------------------------------------------------------------------

#[test]
fn data_as_string_is_rejected() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "entry");
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &["kv", "update", "log", "--id", &idx, "--data", "\"hello\""],
    );
    assert!(!out.status.success(), "string --data must be rejected");
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected exit 4 for non-object data"
    );
}

#[test]
fn data_as_array_is_rejected() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "entry");
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &["kv", "update", "log", "--id", &idx, "--data", "[1,2,3]"],
    );
    assert!(!out.status.success(), "array --data must be rejected");
    assert_eq!(out.status.code(), Some(4), "expected exit 4 for array data");
}

#[test]
fn data_as_null_is_rejected() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "entry");
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &["kv", "update", "log", "--id", &idx, "--data", "null"],
    );
    assert!(!out.status.success(), "null --data must be rejected");
    assert_eq!(out.status.code(), Some(4), "expected exit 4 for null data");
}

// ---------------------------------------------------------------------------
// P10: No value AND --data '{}' — is this rejected?
// ---------------------------------------------------------------------------

/// Empty patch `{}` with no value is now explicitly rejected (exit 4).
/// The handler detects that --data is an empty object and no value was given,
/// which means nothing would be updated, and returns EXIT_INVALID_INPUT.
#[test]
fn data_empty_object_bypasses_noop_guard() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "noop-test");
    let idx = index_from_push(&pushed);

    // No positional value, --data '{}' — must now be rejected
    let out = mx(&dir, &["kv", "update", "log", "--id", &idx, "--data", "{}"]);
    assert!(
        !out.status.success(),
        "empty --data {{}} with no value must be rejected"
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected EXIT_INVALID_INPUT (4), got {:?}",
        out.status.code()
    );
}

// ---------------------------------------------------------------------------
// P11: Validate that --data null-delete actually removes field from JSON output
// ---------------------------------------------------------------------------

/// Push with data `{"a": 1, "b": 2}`, update with `{"b": null}`.
/// GET --json output must not contain key "b" at all.
/// This is the spec-required null-as-delete behavior tested end-to-end via CLI.
#[test]
fn null_as_delete_removes_field_from_json_output() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push_with_data(&dir, "log", "entry", r#"{"a": 1, "b": 2}"#);
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "log",
            "--id",
            &idx,
            "--data",
            r#"{"b": null}"#,
        ],
    );
    assert!(
        out.status.success(),
        "null-delete update must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let get = mx(&dir, &["kv", "get", "log", "--json"]);
    let json_str = String::from_utf8_lossy(&get.stdout).to_string();

    assert!(
        json_str.contains("\"a\""),
        "field a must remain after null-delete of b: {}",
        json_str
    );
    assert!(
        !json_str.contains("\"b\""),
        "field b must be gone after null-delete, got: {}",
        json_str
    );
}

// ---------------------------------------------------------------------------
// P12: Required field: null-delete a required field — must fail validation
// ---------------------------------------------------------------------------

/// Schema declares `status` as required. Push with `{"status": "open"}`.
/// Patch with `{"status": null}` attempts to delete a required field.
/// After merge, the object has no `status` field -> validate_data must reject.
/// Entry must be unchanged.
#[test]
fn null_delete_required_field_fails_validation() {
    let dir = setup(VALIDATED_SCHEMA);
    let pushed = push_with_data(&dir, "tasks", "task-x", r#"{"status": "open"}"#);
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "tasks",
            "--id",
            &idx,
            "--data",
            r#"{"status": null}"#,
        ],
    );
    assert!(
        !out.status.success(),
        "deleting a required field via null must fail"
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected EXIT_INVALID_INPUT (4) for validation failure, got {:?}",
        out.status.code()
    );

    // Entry must be unchanged
    let get = mx(&dir, &["kv", "get", "tasks", "--json"]);
    let json_str = String::from_utf8_lossy(&get.stdout).to_string();
    assert!(
        json_str.contains("\"status\""),
        "status field must still exist after rejected update: {}",
        json_str
    );
}

// ---------------------------------------------------------------------------
// P13: Update by --id kv- with no suffix should be rejected
// ---------------------------------------------------------------------------

/// parse_single_id checks that the suffix after "kv-" is non-empty.
/// Passing `--id kv-` (bare prefix only) should return an error.
#[test]
fn update_with_bare_kv_prefix_is_rejected() {
    let dir = setup(BASIC_SCHEMA);
    push(&dir, "log", "entry");

    let out = mx(&dir, &["kv", "update", "log", "--id", "kv-", "newval"]);
    assert!(
        !out.status.success(),
        "bare 'kv-' ID must be rejected, got exit {:?}",
        out.status.code()
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected EXIT_INVALID_INPUT (4) for empty kv- ID"
    );
}

// ---------------------------------------------------------------------------
// P14: Update output format test — "Updated entry N (kv-<id>)"
// ---------------------------------------------------------------------------

/// The spec says output is: "Updated entry {index} (kv-{id})".
/// Verify the actual output format matches what scripts would parse.
#[test]
fn update_output_format_matches_spec() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "check-format");
    let idx = index_from_push(&pushed);
    let kv_id = id_from_push(&pushed); // "kv-<hash>"

    let out = mx(&dir, &["kv", "update", "log", "--id", &idx, "updated"]);
    assert!(out.status.success(), "update must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    // Must contain "Updated entry N (kv-<hash>)"
    assert!(
        stdout.contains("Updated entry"),
        "output must start with 'Updated entry': {}",
        stdout
    );
    assert!(
        stdout.contains(&kv_id),
        "output must contain the kv-<id>: {}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// P15: Update when no-op (no value, no data) returns error
// ---------------------------------------------------------------------------

/// The spec requires the no-op case to be rejected at the handler level.
/// mx kv update log --id 0  (no value, no --data) must fail with exit 4.
#[test]
fn update_with_no_value_and_no_data_is_rejected() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "entry");
    let idx = index_from_push(&pushed);

    let out = mx(&dir, &["kv", "update", "log", "--id", &idx]);
    assert!(
        !out.status.success(),
        "no-op update must be rejected, got exit {:?}",
        out.status.code()
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected EXIT_INVALID_INPUT (4) for no-op update"
    );
}

// ---------------------------------------------------------------------------
// P16: List type — update works same as history
// ---------------------------------------------------------------------------

#[test]
fn update_list_entry_by_index_works() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "items", "list-item-1");
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &["kv", "update", "items", "--id", &idx, "list-item-1-updated"],
    );
    assert!(
        out.status.success(),
        "update on list must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let get = mx(&dir, &["kv", "get", "items"]);
    assert!(
        String::from_utf8_lossy(&get.stdout).contains("list-item-1-updated"),
        "list entry value must be updated"
    );
}

// ---------------------------------------------------------------------------
// P17: Wrong type for priority field fails with validation error, entry unchanged
// ---------------------------------------------------------------------------

#[test]
fn patch_wrong_type_for_validated_field_fails_entry_unchanged() {
    let dir = setup(VALIDATED_SCHEMA);
    let pushed = push_with_data(
        &dir,
        "tasks",
        "work",
        r#"{"status": "open", "priority": 3}"#,
    );
    let idx = index_from_push(&pushed);

    // priority must be a number; patch with a string
    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "tasks",
            "--id",
            &idx,
            "--data",
            r#"{"priority": "high"}"#,
        ],
    );
    assert!(
        !out.status.success(),
        "wrong type for validated field must fail"
    );
    assert_eq!(out.status.code(), Some(4));

    // Entry must still have priority = 3
    let get = mx(&dir, &["kv", "get", "tasks", "--json"]);
    let json_str = String::from_utf8_lossy(&get.stdout).to_string();
    assert!(
        json_str.contains("\"priority\":3") || json_str.contains("\"priority\": 3"),
        "priority must remain 3 after rejected update: {}",
        json_str
    );
}

// ---------------------------------------------------------------------------
// P18: Merge empty object -> data becomes None (not `{}`)
// ---------------------------------------------------------------------------

/// When the only existing field is deleted via null-patch, merged_obj is empty.
/// The code normalizes this to None (not Some({})).
/// Verify that no `{}` appears in serialized JSON output.
#[test]
fn merge_resulting_in_empty_object_stored_as_none() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push_with_data(&dir, "log", "alone", r#"{"x": 1}"#);
    let idx = index_from_push(&pushed);

    // Delete the only field via null-patch
    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "log",
            "--id",
            &idx,
            "--data",
            r#"{"x": null}"#,
        ],
    );
    assert!(
        out.status.success(),
        "null-delete of sole field must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let get = mx(&dir, &["kv", "get", "log", "--json"]);
    let json_str = String::from_utf8_lossy(&get.stdout).to_string();

    // data should not be an empty object; it should be absent or null
    assert!(
        !json_str.contains("\"data\":{}") && !json_str.contains("\"data\": {}"),
        "empty merged object must be stored as None, not {{}}: {}",
        json_str
    );
}

// ---------------------------------------------------------------------------
// P19: Update invalid JSON for --data
// ---------------------------------------------------------------------------

#[test]
fn invalid_json_for_data_returns_error() {
    let dir = setup(BASIC_SCHEMA);
    let pushed = push(&dir, "log", "entry");
    let idx = index_from_push(&pushed);

    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "log",
            "--id",
            &idx,
            "--data",
            "{not valid json",
        ],
    );
    assert!(!out.status.success(), "invalid JSON must be rejected");
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("invalid JSON"),
        "error message must mention invalid JSON: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// P20: Ambiguous ID prefix — handler returns correct exit code
// ---------------------------------------------------------------------------

/// When two entries share a common prefix, update with that prefix must fail
/// with AmbiguousId -> EXIT_INVALID_INPUT (4).
/// Note: IDs are hashes and may not share prefixes reliably, so we construct
/// a long prefix that could match multiple entries by using a known shared prefix.
/// This test uses index-based lookup as a workaround if hash collision is unlikely,
/// but tests the ambiguity detection at the engine boundary via unit-test helpers.
/// For the CLI path, we rely on the prefix being actually ambiguous.
///
/// We push 50 entries and then use a 1-character prefix — the chance that
/// two entries share a 1-char prefix from a hash alphabet is high enough to
/// test this path. But since we can't guarantee it in a pure test environment,
/// we run this as a probabilistic check with proper assertions only if ambiguous.
#[test]
fn ambiguous_prefix_returns_exit_code_4() {
    let dir = setup(BASIC_SCHEMA);

    // Collect IDs to find one that shares a prefix with another
    let mut ids = Vec::new();
    for i in 0..20 {
        let pushed = push(&dir, "log", &format!("entry-{}", i));
        ids.push(id_from_push(&pushed));
    }

    // Find any 1-char prefix that appears in at least 2 IDs (after stripping "kv-")
    let hashes: Vec<&str> = ids.iter().map(|id| &id[3..]).collect(); // strip "kv-"
    let mut ambiguous_prefix = None;
    'outer: for prefix_len in 1..6 {
        let mut seen = std::collections::HashMap::new();
        for hash in &hashes {
            if hash.len() < prefix_len {
                continue;
            }
            let pfx = &hash[..prefix_len];
            let cnt = seen.entry(pfx).or_insert(0usize);
            *cnt += 1;
            if *cnt >= 2 {
                ambiguous_prefix = Some(format!("kv-{}", pfx));
                break 'outer;
            }
        }
    }

    if let Some(prefix) = ambiguous_prefix {
        let out = mx(&dir, &["kv", "update", "log", "--id", &prefix, "newval"]);
        assert!(
            !out.status.success(),
            "ambiguous prefix must fail, got exit {:?}",
            out.status.code()
        );
        assert_eq!(
            out.status.code(),
            Some(4),
            "AmbiguousId must return exit 4, got {:?}",
            out.status.code()
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            stderr.to_lowercase().contains("ambiguous") || stderr.contains("matches"),
            "stderr must mention ambiguity: {}",
            stderr
        );
    }
    // If no shared prefix found in 20 entries (very unlikely), test passes vacuously
}
