//! CLI-level checks for `mx memory wake-log`, against an isolated, embedded,
//! tempdir-backed store built through `common::isolate`.
//!
//! What only the real binary can show: a spawned `mx` with piped stdout is not
//! a terminal, so the read commands must refuse and print nothing on stdout;
//! `--out` writes the file and prints only its path; `score` with nothing
//! pending prints exactly the counts object. One `#[serial]` test, one store:
//! several processes building embedded stores at once race SurrealDB.

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
