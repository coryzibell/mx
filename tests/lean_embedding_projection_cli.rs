//! CLI-level integration test for the Issue #438 lean embedding projection.
//!
//! Drives the real binary (`CARGO_BIN_EXE_mx`) against an isolated, embedded
//! SurrealDB (`common::isolate`), so it never touches a real store. Covers
//! what a unit test on `SurrealDatabase` can't: the actual CLI flag
//! (`--omit-embedding`) and the end-to-end `--json` default staying
//! byte-shape-identical to today (embedding array present, not null) for an
//! embedded entry.

use serial_test::serial;
use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

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

/// `v[i] = i / 768.0` -- matches the unit-layer fixture
/// (`src/surreal_db/tests.rs::synthetic_768_vector`).
fn synthetic_768_vector() -> Vec<f32> {
    (0..768).map(|i| i as f32 / 768.0).collect()
}

/// `memory seed knowledge` is the only CLI route that writes a vector
/// without an embedding model (`import_jsonl` upserts the file's `embedding`
/// verbatim) -- the only hermetic way to get an embedded fixture into a
/// CLI-level test.
fn write_embedded_fixture(dir: &TempDir) -> std::path::PathBuf {
    let v = synthetic_768_vector();
    let now = chrono::Utc::now().to_rfc3339();
    let line = serde_json::json!({
        "id": "kn-cli-lean-fixture",
        "category_id": "insight",
        "title": "lean projection cli fixture",
        // `updated_at` is `option<datetime>` in the schema with no DEFAULT,
        // and the read projection casts it unconditionally
        // (`<string>updated_at AS updated_at`), which errors on a genuinely
        // absent value -- every production write path already sets it, so
        // this fixture matches that rather than exercising an unrelated
        // pre-existing edge case.
        "updated_at": now,
        "embedding": v,
        "embedding_model": "test-model",
    })
    .to_string();
    let path = dir.path().join("fixture.jsonl");
    std::fs::write(&path, format!("{line}\n")).unwrap();
    path
}

#[test]
#[serial]
fn json_default_stays_full_and_omit_embedding_goes_lean() {
    let dir = TempDir::new().unwrap();
    let fixture = write_embedded_fixture(&dir);

    let seed = mx(
        &dir,
        &["memory", "seed", "knowledge", fixture.to_str().unwrap()],
    );
    assert!(
        seed.status.success(),
        "seed failed: {}",
        String::from_utf8_lossy(&seed.stderr)
    );

    // Default --json: byte-identical to today -- the embedding array is
    // present and matches the fixture vector exactly. This is the contract
    // house consumers (reading `embedding == null` as "unembedded") rely on.
    let out = mx(&dir, &["memory", "list", "--json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    let entries = parsed.as_array().expect("list --json returns an array");
    let entry = entries
        .iter()
        .find(|e| e["id"] == "kn-cli-lean-fixture")
        .expect("the seeded fixture must be in the list output");
    let embedding = entry["embedding"]
        .as_array()
        .expect("default --json must keep the embedding array, not null");
    let expected = synthetic_768_vector();
    assert_eq!(embedding.len(), expected.len());
    for (got, want) in embedding.iter().zip(expected.iter()) {
        assert_eq!(got.as_f64().unwrap() as f32, *want);
    }
    assert!(
        !entry["embedding_model"].is_null(),
        "embedding_model must be populated on the default path"
    );

    // --omit-embedding: the opt-in lean flag emits null, same shape as a
    // never-embedded row, while embedding_model stays populated.
    let out = mx(&dir, &["memory", "list", "--json", "--omit-embedding"]);
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    let entries = parsed.as_array().unwrap();
    let entry = entries
        .iter()
        .find(|e| e["id"] == "kn-cli-lean-fixture")
        .expect("the seeded fixture must still be in the lean list output");
    assert!(
        entry["embedding"].is_null(),
        "--omit-embedding must emit `\"embedding\": null`"
    );
    assert!(
        !entry["embedding_model"].is_null(),
        "--omit-embedding must not touch embedding_model"
    );

    // The underlying stored vector must still be there -- --omit-embedding is
    // an output-shape flag, not a write.
    let out = mx(&dir, &["memory", "show", "kn-cli-lean-fixture", "--json"]);
    assert!(out.status.success());
    let shown: serde_json::Value = serde_json::from_str(&stdout_of(&out)).unwrap();
    assert_eq!(
        shown["embedding"].as_array().map(|a| a.len()),
        Some(768),
        "the stored vector must be intact after a lean --json read elsewhere"
    );
}
