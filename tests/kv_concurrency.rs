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

use std::fs::OpenOptions;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fs2::FileExt;
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

/// A read releases the lock before it touches its output, so a writer runs to
/// completion while the read is still mid-flight.
///
/// The reader is pinned mid-flight by pipe backpressure rather than by a sleep:
/// its stdout is a pipe nobody drains, so once it has written a buffer's worth
/// it blocks in the kernel and stays there. The first byte of output proves it
/// is past `KvStore::from_env`, since `handle_kv` releases the lock before any
/// command arm prints. This fails on the pre-unlock design, where the writer
/// waits out the whole lock timeout and exits non-zero.
#[test]
fn reads_release_the_lock_before_printing() {
    let dir = setup();

    // 20 x 8 KB is ~160 KB of output, comfortably past a 64 KB pipe buffer.
    let big = "x".repeat(8000);
    for _ in 0..20 {
        assert!(run(&dir, &["kv", "push", "race", &big]).status.success());
    }

    let mut reader = cmd(&dir, &["kv", "last", "race", "--count", "20"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn mx");

    let mut first = [0u8; 1];
    reader
        .stdout
        .as_mut()
        .expect("piped stdout")
        .read_exact(&mut first)
        .expect("reader produced no output");

    assert!(
        reader.try_wait().expect("try_wait").is_none(),
        "reader finished instead of blocking; the fixture is too small to fill the pipe"
    );

    let writer = run(&dir, &["kv", "push", "race", "written-during-read"]);
    assert!(
        writer.status.success(),
        "writer did not finish while a read was in flight: {}",
        String::from_utf8_lossy(&writer.stderr)
    );
    assert!(
        reader.try_wait().expect("try_wait").is_none(),
        "reader exited before the writer did; the test proved nothing"
    );

    let out = reader.wait_with_output().expect("reader did not exit");
    assert!(out.status.success());
    assert_eq!(count(&dir, "race"), 21);
}

/// A wedged lock holder makes `mx kv` fail loudly instead of hanging forever.
///
/// A blocking `flock` turns one suspended `mx kv` into a silent freeze of every
/// `mx kv` call on the machine, including the ones on the prompt-submit path.
#[test]
fn a_held_lock_times_out_loudly() {
    let dir = setup();
    assert!(run(&dir, &["kv", "push", "race", "seed"]).status.success());

    let lock = dir.path().join("kv").join("data").join("test.json.lock");
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .expect("lock file should exist after a write");
    held.lock_exclusive().expect("failed to hold the lock");

    let started = Instant::now();
    let blocked = run(&dir, &["kv", "push", "race", "blocked"]);
    let waited = started.elapsed();

    assert!(!blocked.status.success(), "blocked write reported success");
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(stderr.contains("Timed out"), "stderr was: {}", stderr);
    assert!(
        stderr.contains(lock.to_str().expect("utf-8 lock path")),
        "stderr did not name the lock file: {}",
        stderr
    );
    assert!(
        waited < Duration::from_secs(10),
        "waited {:?}; the timeout did not fire",
        waited
    );

    FileExt::unlock(&held).expect("failed to release the lock");
    assert!(run(&dir, &["kv", "push", "race", "after"]).status.success());
    assert_eq!(count(&dir, "race"), 2);
}

/// A command that fails before it can touch the data file leaves nothing behind.
#[test]
fn a_missing_store_is_not_created_by_a_failed_read() {
    let dir = TempDir::new().unwrap();

    let out = run(&dir, &["kv", "get", "nope"]);
    assert!(!out.status.success());

    assert!(
        !dir.path().join("kv").exists(),
        "a failed read created {}",
        dir.path().join("kv").display()
    );
}
