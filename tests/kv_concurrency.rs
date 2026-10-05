//! Integration tests for the kv write lock, and for `kv inc` refusing to lose
//! or fake an increment (#453).
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

[keys.tally]
type = "counter"

[keys.capped]
type = "counter"
max = 3
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

fn counter(dir: &TempDir, key: &str) -> i64 {
    let out = run(dir, &["kv", "get", key]);
    assert!(
        out.status.success(),
        "get {} failed: {}",
        key,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("get {} printed {:?}", key, stdout))
}

/// One line per child: exit code, stdout, stderr. A CI failure has to say WHY.
fn report(outs: &[std::process::Output]) -> String {
    outs.iter()
        .enumerate()
        .map(|(i, o)| {
            format!(
                "  writer {}: exit {:?}, stdout {:?}, stderr {:?}",
                i,
                o.status.code(),
                String::from_utf8_lossy(&o.stdout).trim(),
                String::from_utf8_lossy(&o.stderr).trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every concurrent `kv inc` on one counter lands, and each writer sees its own
/// value: N processes, N increments, printed values exactly 1..=N (#453).
#[test]
fn concurrent_incs_on_one_counter_all_land() {
    let dir = setup();

    let kids: Vec<_> = (0..WRITERS)
        .map(|_| {
            cmd(&dir, &["kv", "inc", "tally"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("failed to spawn mx")
        })
        .collect();

    let outs: Vec<_> = kids
        .into_iter()
        .map(|k| k.wait_with_output().expect("writer did not exit"))
        .collect();
    let report = report(&outs);

    assert!(
        outs.iter().all(|o| o.status.success()),
        "a writer failed:\n{}",
        report
    );

    let mut printed: Vec<i64> = outs
        .iter()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("a writer printed a non-integer:\n{}", report))
        })
        .collect();
    printed.sort_unstable();
    let final_value = counter(&dir, "tally");

    assert_eq!(
        final_value, WRITERS as i64,
        "final value lost increments (printed {:?}):\n{}",
        printed, report
    );
    let expected: Vec<i64> = (1..=WRITERS as i64).collect();
    assert_eq!(
        printed, expected,
        "printed values are not exactly 1..={} (final value {}):\n{}",
        WRITERS, final_value, report
    );
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
    held.lock().expect("failed to hold the lock");

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

    held.unlock().expect("failed to release the lock");
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

// -- `kv inc` refusals (#453): exit code, stderr-only, and nothing written --

fn data_bytes(dir: &TempDir) -> Vec<u8> {
    std::fs::read(dir.path().join("kv").join("data").join("test.json"))
        .expect("data file should exist after a write")
}

/// Run an `inc` that must be refused, then prove the store was not touched.
fn assert_inc_refused(dir: &TempDir, args: &[&str], code: i32, stderr_has: &[&str]) {
    let key = args[2];
    let value_before = counter(dir, key);
    let bytes_before = data_bytes(dir);

    let out = run(dir, args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(code),
        "{:?}: stdout {:?}, stderr {:?}",
        args,
        String::from_utf8_lossy(&out.stdout),
        stderr
    );
    assert!(
        out.stdout.is_empty(),
        "{:?} printed to stdout: {:?}",
        args,
        String::from_utf8_lossy(&out.stdout)
    );
    for needle in stderr_has {
        assert!(
            stderr.contains(needle),
            "{:?}: stderr missing {:?}: {}",
            args,
            needle,
            stderr
        );
    }

    assert_eq!(
        counter(dir, key),
        value_before,
        "{:?} changed the value",
        args
    );
    assert_eq!(
        data_bytes(dir),
        bytes_before,
        "{:?} rewrote the data file",
        args
    );
}

#[test]
fn inc_by_zero_or_negative_is_invalid_input() {
    let dir = setup();
    assert!(run(&dir, &["kv", "set", "tally", "5"]).status.success());

    assert_inc_refused(
        &dir,
        &["kv", "inc", "tally", "--by", "0"],
        4,
        &["must be positive", "got 0", "mx kv dec"],
    );
    assert_inc_refused(
        &dir,
        &["kv", "inc", "tally", "--by", "-1"],
        4,
        &["must be positive", "got -1", "mx kv dec"],
    );
}

#[test]
fn inc_at_max_is_refused() {
    let dir = setup();
    assert!(run(&dir, &["kv", "set", "capped", "3"]).status.success());

    assert_inc_refused(
        &dir,
        &["kv", "inc", "capped"],
        5,
        &[
            "'capped'",
            "is 3",
            "become 3",
            "max is 3",
            "Nothing was written",
        ],
    );
}

#[test]
fn inc_expect_mismatch_is_refused() {
    let dir = setup();
    assert!(run(&dir, &["kv", "set", "tally", "5"]).status.success());

    assert_inc_refused(
        &dir,
        &["kv", "inc", "tally", "--expect", "4"],
        5,
        &["'tally'", "is 5", "expected 4", "Nothing was written"],
    );
}

#[test]
fn inc_expect_match_increments() {
    let dir = setup();
    assert!(run(&dir, &["kv", "set", "tally", "5"]).status.success());

    let out = run(&dir, &["kv", "inc", "tally", "--expect", "5"]);
    assert!(
        out.status.success(),
        "exit {:?}, stderr {:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "6");
    assert_eq!(counter(&dir, "tally"), 6);
}
