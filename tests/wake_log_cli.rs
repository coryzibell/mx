//! CLI-level checks for `mx memory wake-log`, against an isolated, embedded,
//! tempdir-backed store built through `common::isolate`.
//!
//! What only the real binary can show: a spawned `mx` with piped stdout is not
//! a terminal, so the read commands must refuse and print nothing on stdout;
//! `--out` writes the file and prints only its path; `score` with nothing
//! pending prints exactly the counts object; `--out` naming something other
//! than a regular file is refused. Every test is `#[serial]`: several
//! processes building embedded stores at once race SurrealDB.

use serial_test::serial;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");
const AGENT: &str = "agent-a";

const GOODHART_TEXT: &str = "Axis A rises by construction when wake phrases are retuned \
toward logged guesses. It measures how well the phrases fit what the model says. It is a \
tuning dial for phrase authoring and must never be reported as evidence of identity or memory. \
Axis B is the only number here that phrase edits cannot raise. Axis B is evidence of stable \
reaching, not of remembering.";

fn mx_with(dir: &TempDir, agent: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match agent {
        Some(a) => cmd.env("MX_CURRENT_AGENT", a),
        None => cmd.env_remove("MX_CURRENT_AGENT"),
    };
    cmd.output().expect("failed to run mx")
}

fn mx(dir: &TempDir, args: &[&str]) -> Output {
    mx_with(dir, Some(AGENT), args)
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
#[serial]
fn wake_log_read_commands_refuse_piped_stdout_end_to_end() {
    let dir = TempDir::new().unwrap();

    // ---- the Goodhart text is in report --help --------------------------
    let out = mx(&dir, &["memory", "wake-log", "report", "--help"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        squash(&stdout_of(&out)).contains(&squash(GOODHART_TEXT)),
        "{}",
        stdout_of(&out)
    );

    // ---- piped stdout is not a terminal: refused, nothing on stdout -------
    for args in [
        vec!["memory", "wake-log", "report"],
        vec!["memory", "wake-log", "report", "--wake", "1"],
        vec!["memory", "wake-log", "bloom", "kn-invented"],
        vec!["memory", "wake-log", "wake", "1"],
    ] {
        let out = mx(&dir, &args);
        assert!(!out.status.success(), "{args:?} must be refused");
        assert!(
            out.stdout.is_empty(),
            "{args:?} printed {:?}",
            stdout_of(&out)
        );
        let err = stderr_of(&out);
        assert!(err.contains("terminal-only"), "{args:?}: {err}");
        assert!(err.contains("--out FILE"), "{args:?}: {err}");
    }

    // ---- --out writes the file and prints only its path -------------------
    let json_path = dir.path().join("wake-1.json");
    let json_str = json_path.to_str().unwrap();
    let out = mx(
        &dir,
        &["memory", "wake-log", "wake", "1", "--out", json_str],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert_eq!(stdout_of(&out), format!("{json_str}\n"));
    let body = std::fs::read_to_string(&json_path).unwrap();
    assert!(body.starts_with("{\n  \"goodhart\": "), "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["goodhart"], GOODHART_TEXT);
    assert_eq!(v["rows"], serde_json::json!([]));

    let text_path = dir.path().join("report.txt");
    let text_str = text_path.to_str().unwrap();
    let out = mx(
        &dir,
        &[
            "memory", "wake-log", "report", "--wake", "1", "--out", text_str,
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert_eq!(stdout_of(&out), format!("{text_str}\n"));
    let body = std::fs::read_to_string(&text_path).unwrap();
    assert!(body.starts_with(&format!("{GOODHART_TEXT}\n\n")), "{body}");
    assert!(body.contains("scored 0/0"), "{body}");

    // ---- score with nothing pending: counts only, no model ----------------
    let out = mx(&dir, &["memory", "wake-log", "score"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert_eq!(
        stdout_of(&out),
        "{\"status\":\"scored\",\"rows\":0,\"skipped\":0}\n"
    );

    // ---- no calling agent: refused, nothing on stdout ---------------------
    for args in [
        vec!["memory", "wake-log", "score"],
        vec!["memory", "wake-log", "wake", "1", "--out", json_str],
    ] {
        let out = mx_with(&dir, None, &args);
        assert!(!out.status.success(), "{args:?} must be refused");
        assert!(
            out.stdout.is_empty(),
            "{args:?} printed {:?}",
            stdout_of(&out)
        );
        assert!(
            stderr_of(&out).contains("MX_CURRENT_AGENT"),
            "{}",
            stderr_of(&out)
        );
    }
}

/// `--out /dev/stdout` and its kin would turn a refused pipe back into stdout,
/// so an `--out` path that exists must be a regular file.
#[test]
#[serial]
#[cfg(unix)]
fn cli_out_dev_stdout_does_not_bypass_the_terminal_rule() {
    let dir = TempDir::new().unwrap();
    // /proc exists only on Linux; elsewhere /dev/stdout covers the same path.
    let targets: &[&str] = if cfg!(target_os = "linux") {
        &["/dev/stdout", "/proc/self/fd/1"]
    } else {
        &["/dev/stdout"]
    };
    for &target in targets {
        let out = mx(
            &dir,
            &[
                "memory", "wake-log", "report", "--wake", "1", "--out", target,
            ],
        );
        assert!(
            !stdout_of(&out).contains("Axis A rises by construction"),
            "--out {target} printed the report into a pipe: {:?}",
            stdout_of(&out)
        );
        assert!(!out.status.success(), "--out {target} must be refused");
        assert!(out.stdout.is_empty(), "{:?}", stdout_of(&out));
        let err = stderr_of(&out);
        assert!(err.contains("--out must name a regular file"), "{err}");
    }
}

/// A model's shell tool may capture stdout in a regular file, not a pipe;
/// `--out` naming that same file must still be refused.
#[test]
#[serial]
#[cfg(unix)]
fn cli_out_naming_stdout_captured_in_a_file_is_refused() {
    let dir = TempDir::new().unwrap();
    let cap_dir = TempDir::new().unwrap();
    let capture = cap_dir.path().join("capture.txt");
    // /proc exists only on Linux; elsewhere /dev/stdout covers the same path.
    let targets: &[&str] = if cfg!(target_os = "linux") {
        &["/dev/stdout", "/proc/self/fd/1", "/dev/stderr"]
    } else {
        &["/dev/stdout", "/dev/stderr"]
    };
    for &target in targets {
        let file = std::fs::File::create(&capture).unwrap();
        let mut cmd = Command::new(MX);
        common::isolate(&mut cmd, dir.path());
        cmd.args([
            "memory", "wake-log", "report", "--wake", "1", "--out", target,
        ])
        .env("MX_CURRENT_AGENT", AGENT)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file));
        let status = cmd.status().expect("failed to run mx");
        let captured = std::fs::read_to_string(&capture).unwrap();
        assert!(
            !status.success(),
            "--out {target} must be refused: {captured:?}"
        );
        assert!(
            !captured.contains("Axis A rises by construction"),
            "--out {target}: {captured:?}"
        );
        assert!(
            captured.contains("--out names this process's own stdout or stderr"),
            "{captured}"
        );
    }
}

/// `/proc/self/fd/N` for an fd that is not open yet passes the check (the path
/// does not exist), then names the store's own files once the DB is open. The
/// report must never be written into the store.
#[test]
#[serial]
#[cfg(target_os = "linux")]
fn cli_out_proc_self_fd_never_writes_into_the_store() {
    for n in 3..=24 {
        let dir = TempDir::new().unwrap();
        let seed = dir.path().join("seed.json");
        let out = mx(
            &dir,
            &[
                "memory",
                "wake-log",
                "wake",
                "1",
                "--out",
                seed.to_str().unwrap(),
            ],
        );
        assert!(out.status.success(), "seed: {}", stderr_of(&out));

        let target = format!("/proc/self/fd/{n}");
        let attempt = mx(&dir, &["memory", "wake-log", "wake", "1", "--out", &target]);
        assert!(!attempt.status.success(), "--out {target} must be refused");
        assert!(
            attempt.stdout.is_empty(),
            "--out {target}: {:?}",
            stdout_of(&attempt)
        );
        assert!(
            stderr_of(&attempt).contains("so nothing was read or printed"),
            "--out {target}: {}",
            stderr_of(&attempt)
        );

        let check = dir.path().join("check.json");
        let after = mx(
            &dir,
            &[
                "memory",
                "wake-log",
                "wake",
                "1",
                "--out",
                check.to_str().unwrap(),
            ],
        );
        assert!(
            after.status.success(),
            "--out {target} (exit {:?}, stdout {:?}) left the store unopenable: {}",
            attempt.status.code(),
            stdout_of(&attempt),
            stderr_of(&after)
        );
    }
}

/// The `--out` file is opened before anything is read but emptied only when
/// the output is written: a call refused after the open leaves an existing
/// file as it was, and a call that succeeds replaces all of it.
#[test]
#[serial]
fn cli_out_existing_file_is_kept_when_refused_and_replaced_on_success() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("old.txt");
    let file_s = file.to_str().unwrap();
    let old = "invented earlier notes\n".repeat(4096);
    std::fs::write(&file, &old).unwrap();

    let no_agent = mx_with(
        &dir,
        None,
        &["memory", "wake-log", "wake", "1", "--out", file_s],
    );
    assert!(!no_agent.status.success());
    assert!(no_agent.stdout.is_empty(), "{:?}", stdout_of(&no_agent));
    let no_scored_wake = mx(&dir, &["memory", "wake-log", "report", "--out", file_s]);
    assert!(!no_scored_wake.status.success());
    assert!(
        stderr_of(&no_scored_wake).contains("no scored wake"),
        "{}",
        stderr_of(&no_scored_wake)
    );
    assert!(
        no_scored_wake.stdout.is_empty(),
        "{:?}",
        stdout_of(&no_scored_wake)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), old);

    let ok = mx(&dir, &["memory", "wake-log", "wake", "1", "--out", file_s]);
    assert!(ok.status.success(), "{}", stderr_of(&ok));
    assert_eq!(stdout_of(&ok), format!("{file_s}\n"));
    let body = std::fs::read_to_string(&file).unwrap();
    assert!(squash(&body).starts_with(&squash(GOODHART_TEXT)), "{body}");
    assert!(!body.contains("invented earlier notes"), "{body}");
}

/// Opening a FIFO for writing blocks until a reader appears, so an `--out`
/// FIFO is refused before it is opened, and the call never hangs.
#[test]
#[serial]
#[cfg(unix)]
fn cli_out_fifo_is_refused_without_hanging() {
    use std::time::{Duration, Instant};
    let dir = TempDir::new().unwrap();
    let fifo = dir.path().join("pipe.txt");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args([
        "memory",
        "wake-log",
        "report",
        "--wake",
        "1",
        "--out",
        fifo.to_str().unwrap(),
    ])
    .env("MX_CURRENT_AGENT", AGENT)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to run mx");
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("--out a FIFO hung");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().unwrap();
    let err = stderr_of(&out);
    assert!(!out.status.success(), "--out a FIFO must be refused");
    assert!(out.stdout.is_empty(), "{:?}", stdout_of(&out));
    assert!(err.contains("--out must name a regular file"), "{err}");
}
