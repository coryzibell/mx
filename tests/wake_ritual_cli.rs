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
            "7",
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

// =========================================================================
// Yeet's admitted gaps 2 and 3: the DEFAULT three-layer cascade and
// `--include-excluded` driven through the real binary rather than the
// min-resonance path and the query layer.
// =========================================================================

/// The default cascade (no `--min-resonance`) is the path every real wake
/// takes, and until now only the min-resonance path was driven end to end.
#[test]
#[serial]
fn the_default_cascade_opens_a_ritual_and_honours_the_exclusion() {
    let dir = TempDir::new().unwrap();
    add_bloom(&dir, "Cascade Alpha", "Alpha body text.", "alpha cue", None);
    add_bloom(&dir, "Cascade Beta", "Beta body text.", "beta cue", None);
    add_bloom(
        &dir,
        "Cascade Shelved",
        "Shelved body text.",
        "shelved cue",
        Some("wake-exclude"),
    );

    let out = mx(
        &dir,
        &["memory", "wake", "--begin", "--model", "test-model"],
    );
    assert!(
        out.status.success(),
        "the default cascade must open a ritual; stderr: {}",
        stderr_of(&out)
    );
    let begin = json_of(&out);
    assert_eq!(begin["status"], "ritual_started");
    assert_eq!(
        begin["progress"]["bloom_total"], 2,
        "the excluded entry must not be in the default cascade: {begin}"
    );
    assert_eq!(begin["excluded"]["wake-exclude"], 1);

    // And the override brings it back through --begin, not just through the
    // listing and the query layer.
    let out = mx(&dir, &["memory", "wake", "--begin", "--include-excluded"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let begin = json_of(&out);
    assert_eq!(
        begin["progress"]["bloom_total"], 3,
        "--include-excluded must restore the entry through --begin: {begin}"
    );
    // The key is always present, so this reads as an empty object rather than
    // an absent key — one shape for the consumer either way.
    assert_eq!(
        begin["excluded"],
        serde_json::json!({}),
        "nothing is excluded when the override is on: {begin}"
    );
}

/// Two `--begin` calls over unchanged data must produce the same sequence.
#[test]
#[serial]
fn two_begins_over_the_same_data_produce_the_same_sequence() {
    let dir = TempDir::new().unwrap();
    for (title, cue) in [
        ("Order One", "one cue"),
        ("Order Two", "two cue"),
        ("Order Three", "three cue"),
    ] {
        add_bloom(&dir, title, "Body text.", cue, None);
    }

    let sequence = |dir: &TempDir| -> Vec<String> {
        let mut titles = Vec::new();
        let out = mx(dir, &["memory", "wake", "--begin"]);
        assert!(out.status.success(), "stderr: {}", stderr_of(&out));
        let mut node = json_of(&out);
        let mut token = node["session"].as_str().unwrap().to_string();
        let mut id = node["prompt"]["id"].as_str().unwrap().to_string();
        titles.push(node["prompt"]["title"].as_str().unwrap().to_string());
        loop {
            let out = mx(
                dir,
                &[
                    "memory",
                    "wake",
                    "--bloom-id",
                    &id,
                    "--respond",
                    "no match at all",
                    "--session",
                    &token,
                ],
            );
            assert!(out.status.success(), "stderr: {}", stderr_of(&out));
            node = json_of(&out);
            let Some(next) = node.get("next").filter(|n| !n.is_null()) else {
                break;
            };
            titles.push(next["title"].as_str().unwrap().to_string());
            id = next["id"].as_str().unwrap().to_string();
            token = node["session"].as_str().unwrap().to_string();
        }
        titles
    };

    let first = sequence(&dir);
    let second = sequence(&dir);
    assert_eq!(first.len(), 3, "precondition: all three blooms walked");
    assert_eq!(
        first, second,
        "the wake sequence is not stable across begins"
    );
}

/// Gap 4: `chunk_truncated` was only ever driven against the mock store. This
/// walks it through the real binary and a real store: a chunked bloom is
/// shrunk under the session's cursor mid-ritual, and the ritual must report
/// the truncation and keep going.
#[test]
#[serial]
fn a_bloom_shrunk_mid_ritual_reports_chunk_truncated_end_to_end() {
    /// `mx` with a small chunk threshold, so a modest fixture chunks.
    fn mx_chunked(dir: &TempDir, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(MX);
        common::isolate(&mut cmd, dir.path());
        cmd.args(args)
            .env("MX_CURRENT_AGENT", AGENT)
            .env("MX_WAKE_CHUNK_BYTES", "300")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn mx");
        drop(child.stdin.take());
        child.wait_with_output().expect("failed to wait on mx")
    }

    let dir = TempDir::new().unwrap();
    let mut body = String::new();
    for section in 1..=6 {
        body.push_str(&format!(
            "\n## Section {section}\n\nThis is section {section} of the fixture, \
             long enough that the sections cross the chunking threshold.\n\n"
        ));
    }
    add_bloom(&dir, "Chunky Note", &body, "chunky cue", None);
    add_bloom(&dir, "Tail Note", "Tail body text.", "tail cue", None);

    let out = mx_chunked(&dir, &["memory", "wake", "--begin"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let begin = json_of(&out);
    let total = begin["progress"]["total"].as_u64().unwrap();
    assert!(
        total > 2,
        "precondition: the fixture must chunk, got {total}"
    );

    // The sequence is ordered by the cascade, not by insertion, so walk
    // forward until the chunked bloom is the one on the table, then take one
    // more chunk of it so the cursor sits past chunk 0.
    let mut id = begin["prompt"]["id"].as_str().unwrap().to_string();
    let mut title = begin["prompt"]["title"].as_str().unwrap().to_string();
    let mut token = begin["session"].as_str().unwrap().to_string();
    let mut chunky_id = String::new();

    for _ in 0..total + 1 {
        let out = mx_chunked(
            &dir,
            &[
                "memory",
                "wake",
                "--bloom-id",
                &id,
                "--respond",
                "no match",
                "--session",
                &token,
            ],
        );
        assert!(out.status.success(), "stderr: {}", stderr_of(&out));
        let node = json_of(&out);
        token = node["session"].as_str().unwrap().to_string();
        if title.starts_with("Chunky") {
            chunky_id = id.clone();
            break;
        }
        let next = node.get("next").expect("more blooms to walk");
        id = next["id"].as_str().unwrap().to_string();
        title = next["title"].as_str().unwrap().to_string();
    }
    assert!(
        !chunky_id.is_empty(),
        "precondition: the chunked bloom must be reachable"
    );

    let out = mx_chunked(
        &dir,
        &["memory", "update", &chunky_id, "--content", "tiny now."],
    );
    assert!(out.status.success(), "update; stderr: {}", stderr_of(&out));

    let out = mx_chunked(
        &dir,
        &[
            "memory",
            "wake",
            "--bloom-id",
            &chunky_id,
            "--respond",
            "ignored",
            "--session",
            &token,
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let trunc = json_of(&out);
    assert_eq!(
        trunc["status"], "chunk_truncated",
        "a bloom shrunk under the cursor must report the truncation: {trunc}"
    );
    assert!(
        trunc.get("bucket").is_none() && trunc.get("guess").is_none(),
        "no guess was judged, so neither is reported: {trunc}"
    );
    assert!(
        trunc.get("next").is_some() || trunc.get("summary").is_some(),
        "a truncation must still hand back a next prompt or a summary: {trunc}"
    );
}

// =========================================================================
// Adversarial: flags and error surfaces.
// =========================================================================

/// `--wake` and `--model` are declared `requires = "begin"`, but
/// `--include-excluded` is not, so a respond call accepts a wake-set flag it
/// cannot act on and silently ignores it. The same is true of `--limit`,
/// `--min-resonance`, `--days` and `--no-activate`: the respond branch never
/// reads any of them.
#[test]
#[serial]
fn wake_set_flags_are_rejected_on_a_respond_call() {
    let dir = TempDir::new().unwrap();
    add_bloom(&dir, "Flag Alpha", "Alpha body text.", "alpha cue", None);

    let out = mx(&dir, &["memory", "wake", "--begin"]);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let begin = json_of(&out);
    let token = begin["session"].as_str().unwrap().to_string();
    let id = begin["prompt"]["id"].as_str().unwrap().to_string();

    for flag in [
        vec!["--include-excluded"],
        vec!["--min-resonance", "9"],
        vec!["--limit", "5"],
    ] {
        let mut args = vec![
            "memory",
            "wake",
            "--bloom-id",
            &id,
            "--respond",
            "alpha cue",
            "--session",
            &token,
        ];
        args.extend(flag.iter().copied());
        let out = mx(&dir, &args);
        assert!(
            !out.status.success(),
            "{flag:?} was accepted and silently ignored on a respond call; stdout: {}",
            stdout_of(&out)
        );
    }
}

/// With the exclusion on by default, tagging the wrong thing empties the wake
/// set. The failure a user meets then is a bare "No blooms to wake", which
/// names neither the exclusion nor the flag that turns it off — and the
/// entries are sitting right there in the graph.
#[test]
#[serial]
fn an_all_excluded_wake_set_says_why_it_is_empty() {
    let dir = TempDir::new().unwrap();
    add_bloom(
        &dir,
        "Shelved One",
        "Body text.",
        "one cue",
        Some("archive"),
    );
    add_bloom(
        &dir,
        "Shelved Two",
        "Body text.",
        "two cue",
        Some("wake-exclude"),
    );

    let out = mx(&dir, &["memory", "wake", "--begin"]);
    assert!(!out.status.success(), "an empty wake set must fail");

    // Precondition: the entries are there, and the override finds them.
    let check = mx(&dir, &["memory", "wake", "--begin", "--include-excluded"]);
    assert!(
        check.status.success(),
        "precondition: the entries exist; stderr: {}",
        stderr_of(&check)
    );

    let err = stderr_of(&out);
    assert!(
        err.contains("exclude") || err.contains("archive"),
        "the error must name the exclusion that emptied the set, got: {err}"
    );
}

/// Section 3 retires this vocabulary from tool output, help text and error
/// messages alike.
#[test]
#[serial]
fn no_retired_vocabulary_survives_in_the_wake_help_text() {
    let dir = TempDir::new().unwrap();
    let mut seen = String::new();
    for args in [
        vec!["memory", "wake", "--help"],
        vec!["memory", "add", "--help"],
        vec!["memory", "update", "--help"],
        vec!["memory", "--help"],
    ] {
        seen.push_str(&stdout_of(&mx(&dir, &args)));
    }
    for retired in [
        "remembered",
        "needed_help",
        "needed help",
        "incorrect",
        "prove",
        "verification",
    ] {
        assert!(
            !seen.to_lowercase().contains(retired),
            "help text still carries the retired word {retired:?}"
        );
    }
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
