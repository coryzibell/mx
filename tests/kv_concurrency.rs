//! Integration test for the kv write lock.
//!
//! `mx kv` loads the whole data file, mutates it in memory, and rewrites the
//! whole thing. Without a lock spanning that cycle, two overlapping writers each
//! save a snapshot taken before the other's push, and one entry vanishes. The
//! doors hook (Issue: `mx doors hook`) writes on the UserPromptSubmit path,
//! which is exactly when a subagent may be mid-`mx kv push`, so this stopped
//! being theoretical.
//!
//! Harness mirrors `tests/kv_set_cli.rs`: the built binary against an isolated
//! MX_HOME so nothing touches the live store.

use std::process::Command;
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const SCHEMA: &str = r#"
[keys.race]
type = "history"
max_entries = 1000

[keys.other]
type = "history"
max_entries = 1000
"#;

const WRITERS: usize = 8;

fn setup() -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), SCHEMA).unwrap();
    dir
}

fn cmd(dir: &TempDir, args: &[&str]) -> Command {
    let mut c = Command::new(MX);
    common::isolate(&mut c, dir.path());
    c.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA");
    c
}

fn run(dir: &TempDir, args: &[&str]) -> std::process::Output {
    cmd(dir, args).output().expect("failed to run mx")
}

fn count(dir: &TempDir, key: &str) -> usize {
    let out = run(dir, &["kv", "count", key]);
    assert!(
        out.status.success(),
        "count {} failed: {}",
        key,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("count {} printed {:?}", key, stdout))
}

/// Every concurrent `kv push` to one key survives.
#[test]
fn concurrent_pushes_to_one_key_all_land() {
    let dir = setup();
    run(&dir, &["kv", "push", "race", "seed"]);

    let mut kids: Vec<_> = (0..WRITERS)
        .map(|i| {
            let value = format!("v{}", i);
            cmd(&dir, &["kv", "push", "race", &value])
                .spawn()
                .expect("failed to spawn mx")
        })
        .collect();

    for kid in &mut kids {
        let status = kid.wait().expect("writer did not exit");
        assert!(status.success(), "writer exited {:?}", status.code());
    }

    assert_eq!(count(&dir, "race"), WRITERS + 1);
}

/// The doors case: a writer on one key must not clobber a writer on another.
#[test]
fn concurrent_pushes_to_different_keys_all_land() {
    let dir = setup();
    run(&dir, &["kv", "push", "race", "seed"]);
    run(&dir, &["kv", "push", "other", "seed"]);

    let mut kids: Vec<_> = (0..WRITERS)
        .map(|i| {
            let key = if i % 2 == 0 { "race" } else { "other" };
            let value = format!("v{}", i);
            cmd(&dir, &["kv", "push", key, &value])
                .spawn()
                .expect("failed to spawn mx")
        })
        .collect();

    for kid in &mut kids {
        let status = kid.wait().expect("writer did not exit");
        assert!(status.success(), "writer exited {:?}", status.code());
    }

    assert_eq!(count(&dir, "race"), WRITERS / 2 + 1);
    assert_eq!(count(&dir, "other"), WRITERS / 2 + 1);
}

/// A read command releases the lock before it does anything else, so it cannot
/// wedge a concurrent writer.
#[test]
fn reads_do_not_block_writes() {
    let dir = setup();
    run(&dir, &["kv", "push", "race", "seed"]);

    let mut readers: Vec<_> = (0..WRITERS)
        .map(|_| {
            cmd(&dir, &["kv", "last", "race"])
                .spawn()
                .expect("failed to spawn mx")
        })
        .collect();
    let mut writers: Vec<_> = (0..WRITERS)
        .map(|i| {
            let value = format!("v{}", i);
            cmd(&dir, &["kv", "push", "race", &value])
                .spawn()
                .expect("failed to spawn mx")
        })
        .collect();

    for kid in readers.iter_mut().chain(writers.iter_mut()) {
        assert!(kid.wait().expect("child did not exit").success());
    }

    assert_eq!(count(&dir, "race"), WRITERS + 1);
}
