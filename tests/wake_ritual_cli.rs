//! CLI-level integration test for the one-guess wake ritual.
//!
//! This drives the real binary (`CARGO_BIN_EXE_mx`) against an isolated,
//! embedded, tempdir-backed SurrealDB via `common::isolate`, so it never
//! touches a real store. What it pins that the unit layer cannot:
//!
//!   - a `--respond` call does not change any entry's activation count
//!     (the cascade used to be re-run, and re-counted, on every guess);
//!   - guess rows never appear in any `memory export` format;
//!   - `--wake`, `--model` and `--include-excluded` are wired end to end;
//!   - `--skip` is gone from the parser.
//!
//! Everything runs inside ONE `#[serial]` test against ONE seeded store:
//! spinning up an embedded SurrealDB applies the full schema, and doing that
//! from several test processes at once races SurrealDB's optimistic
//! concurrency. All fixture content here is invented.

use serial_test::serial;
use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");
const AGENT: &str = "agent-a";

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", AGENT)
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
fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    let text = stdout_of(out);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("expected JSON on stdout, got {text:?} ({e})"))
}

/// Add a public bloom with one authored phrase. `--no-embed` keeps the ONNX
/// embedding model (a network- and cache-dependent side effect) out of a
/// hermetic test.
fn add_bloom(dir: &TempDir, title: &str, body: &str, phrase: &str, tags: Option<&str>) {
    let mut args = vec![
        "memory",
        "add",
        "--category",
        "bloom",
        "--title",
        title,
        "--content",
        body,
        "--resonance",
        "9",
        "--resonance-type",
        "foundational",
        "--wake-phrase",
        phrase,
        "--no-embed",
        "--no-auto-anchor",
    ];
    if let Some(tags) = tags {
        args.push("--tags");
        args.push(tags);
    }
    let out = mx(dir, &args);
    assert!(
        out.status.success(),
        "add {title:?} must succeed; stderr: {}",
        stderr_of(&out)
    );
}

/// Every exported entry's `(title, activation_count)`, read out of the JSONL
/// export because that format carries the whole entry.
fn activation_counts(dir: &TempDir) -> Vec<(String, i64)> {
    let path = dir.path().join("export.jsonl");
    let path_str = path.to_str().unwrap();
    let out = mx(
        dir,
        &[
            "memory", "export", "--format", "jsonl", "--output", path_str,
        ],
    );
    assert!(
        out.status.success(),
        "jsonl export must succeed; stderr: {}",
        stderr_of(&out)
    );
    let text = std::fs::read_to_string(&path).expect("export file");
    let mut counts: Vec<(String, i64)> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).expect("jsonl line");
            (
                v["title"].as_str().unwrap_or_default().to_string(),
                v["activation_count"].as_i64().unwrap_or_default(),
            )
        })
        .collect();
    counts.sort();
    counts
}

#[test]
#[serial]
fn one_guess_ritual_end_to_end() {
    let dir = TempDir::new().unwrap();

    // Two live blooms and one tagged for exclusion.
    add_bloom(&dir, "Alpha Note", "Alpha body text.", "alpha cue", None);
    add_bloom(&dir, "Beta Note", "Beta body text.", "beta cue", None);
    add_bloom(
        &dir,
        "Shelved Note",
        "Shelved body text.",
        "shelved cue",
        Some("archive"),
    );

    // ---- --skip is gone from the parser (#452) --------------------------
    let out = mx(&dir, &["memory", "wake", "--skip"]);
    assert!(!out.status.success(), "--skip must no longer parse");
    let err = stderr_of(&out);
    assert!(
        err.contains("unexpected argument") || err.contains("--skip"),
        "clap should reject --skip as unknown; stderr: {err}"
    );

    // ---- the excluded bloom is out of the plain wake set -----------------
    let out = mx(&dir, &["memory", "wake", "--min-resonance", "9"]);
    assert!(
        out.status.success(),
        "plain wake must succeed; stderr: {}",
        stderr_of(&out)
    );
    let listing = stdout_of(&out);
    assert!(listing.contains("Alpha Note"), "{listing}");
    assert!(
        !listing.contains("Shelved Note"),
        "an archive-tagged entry is out of the wake set by default: {listing}"
    );

    let out = mx(
        &dir,
        &[
            "memory",
            "wake",
            "--min-resonance",
            "9",
            "--include-excluded",
        ],
    );
    assert!(
        stdout_of(&out).contains("Shelved Note"),
        "--include-excluded brings it back: {}",
        stdout_of(&out)
    );

    // ---- begin the ritual ------------------------------------------------
    let out = mx(
        &dir,
        &[
            "memory",
            "wake",
            "--min-resonance",
            "9",
            "--begin",
            "--wake",
            "463",
            "--model",
            "test-model",
        ],
    );
    assert!(
        out.status.success(),
        "--begin must succeed; stderr: {}",
        stderr_of(&out)
    );
    let begin = json_of(&out);
    assert_eq!(begin["status"], "ritual_started");
    assert_eq!(begin["excluded"]["archive"], 1);
    assert_eq!(begin["progress"]["bloom_total"], 2);
    let prompt_title = begin["prompt"]["title"].as_str().unwrap().to_string();
    assert!(
        !prompt_title.contains("Shelved"),
        "the excluded bloom must not open the ritual: {prompt_title}"
    );

    // The begin call is the one that counts as surfacing the entries.
    let after_begin = activation_counts(&dir);
    assert!(
        after_begin.iter().any(|(_, c)| *c > 0),
        "begin should have activated the cascade: {after_begin:?}"
    );

    // ---- one guess per bloom --------------------------------------------
    let mut token = begin["session"].as_str().unwrap().to_string();
    let mut bloom_id = begin["prompt"]["id"].as_str().unwrap().to_string();

    // First bloom: a guess that misses.
    let out = mx(
        &dir,
        &[
            "memory",
            "wake",
            "--bloom-id",
            &bloom_id,
            "--respond",
            "nothing like the cue",
            "--session",
            &token,
        ],
    );
    assert!(
        out.status.success(),
        "--respond must succeed; stderr: {}",
        stderr_of(&out)
    );
    let first = json_of(&out);
    assert_eq!(first["status"], "shown");
    assert_eq!(first["bucket"], "revealed");
    assert_eq!(first["match"]["kind"], "none");
    assert!(
        first["bloom"]["content"].as_str().unwrap().contains("body"),
        "the bloom is shown after the one guess: {first}"
    );

    // A respond must not re-count activations (#451).
    let after_respond = activation_counts(&dir);
    assert_eq!(
        after_begin, after_respond,
        "a --respond call must not change any activation count"
    );

    // Second bloom: the cue, matched.
    token = first["session"].as_str().unwrap().to_string();
    bloom_id = first["next"]["id"].as_str().unwrap().to_string();
    let cue = if first["next"]["title"] == "Alpha Note" {
        "alpha cue"
    } else {
        "beta cue"
    };
    let out = mx(
        &dir,
        &[
            "memory",
            "wake",
            "--bloom-id",
            &bloom_id,
            "--respond",
            cue,
            "--session",
            &token,
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let second = json_of(&out);
    assert_eq!(second["bucket"], "unhinted");
    assert_eq!(second["match"]["kind"], "exact");
    let summary = &second["summary"];
    assert_eq!(summary["chunks"], 2);
    assert_eq!(summary["blooms"], 2);
    assert_eq!(summary["buckets"]["unhinted"]["authored"], 1);
    assert_eq!(summary["buckets"]["revealed"]["authored"], 1);

    assert_eq!(
        after_begin,
        activation_counts(&dir),
        "the second --respond must not change any activation count either"
    );

    // ---- guess rows stay out of every export format ----------------------
    // `memory export` reads the knowledge table only. Pin that: a guess this
    // ritual just logged must not surface in any format.
    let guess_text = "nothing like the cue";

    let jsonl = dir.path().join("check.jsonl");
    let out = mx(
        &dir,
        &[
            "memory",
            "export",
            "--format",
            "jsonl",
            "--output",
            jsonl.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let jsonl_text = std::fs::read_to_string(&jsonl).unwrap();
    assert!(
        jsonl_text.contains("Alpha Note"),
        "precondition: the export must contain entries at all"
    );
    assert!(
        !jsonl_text.contains(guess_text) && !jsonl_text.contains("wake_guess"),
        "jsonl export leaked the guess log"
    );

    let csv = dir.path().join("check.csv");
    let out = mx(
        &dir,
        &[
            "memory",
            "export",
            "--format",
            "csv",
            "--output",
            csv.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let csv_text = std::fs::read_to_string(&csv).unwrap();
    assert!(
        !csv_text.contains(guess_text) && !csv_text.contains("wake_guess"),
        "csv export leaked the guess log"
    );

    let md_dir = dir.path().join("md-export");
    let out = mx(
        &dir,
        &[
            "memory",
            "export",
            "--format",
            "md",
            "--output",
            md_dir.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let mut md_text = String::new();
    for entry in walk_files(&md_dir) {
        md_text.push_str(&std::fs::read_to_string(entry).unwrap_or_default());
    }
    assert!(
        md_text.contains("Alpha"),
        "precondition: the md export must contain entries at all"
    );
    assert!(
        !md_text.contains(guess_text) && !md_text.contains("wake_guess"),
        "md export leaked the guess log"
    );
}

/// Every file under `dir`, recursively. Returns nothing if `dir` is missing.
fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}
