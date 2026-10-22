//! Integration tests for doors P2: `--scope`, `mx doors mode`, hints,
//! `place` on fire rows, `check --mode` and `MX_DOORS_BUDGET`.
//!
//! Same isolated-store harness as `tests/doors_cli.rs`. Fixtures are public
//! domain (Project Gutenberg) or synthetic; book slugs are `walden`, `prince`
//! and `liberty` only.

use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

const WALDEN: &str = include_str!("fixtures/doors/pd/walden.txt");
const CV: &str = include_str!("fixtures/doors/syn/syn-cv.txt");

const FACTS_AND_FIRED: &str = r#"
[keys.facts]
type = "list"

[keys.doors_fired]
type = "history"
max_entries = 5000

[keys.doors_fired.data.session]
type = "string"
required = true

[keys.doors_fired.data.key]
type = "string"
required = true

[keys.doors_fired.data.entry]
type = "string"
required = true

[keys.doors_fired.data.trigger]
type = "string"

[keys.doors_fired.data.origin]
type = "string"
"#;

const PLACE_FIELD: &str = r#"
[keys.doors_fired.data.place]
type = "string"
required = false
"#;

const SESSION_KEY: &str = r#"
[keys.doors_session]
type = "history"
max_entries = 1000

[keys.doors_session.data.session]
type = "string"
required = true

[keys.doors_session.data.event]
type = "string"
required = true

[keys.doors_session.data.mode]
type = "string"

[keys.doors_session.data.by]
type = "string"
"#;

const KIND_FIELD: &str = r#"
[keys.doors_session.data.kind]
type = "string"
"#;

/// The full P2 schema: fire rows with `place`, and `doors_session`.
fn setup() -> TempDir {
    setup_schema(&[FACTS_AND_FIRED, PLACE_FIELD, SESSION_KEY, KIND_FIELD])
}

fn setup_schema(parts: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let schema_dir = dir.path().join("kv").join("schema");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::write(schema_dir.join("test.toml"), parts.concat()).unwrap();
    dir
}

fn run(dir: &TempDir, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "test")
        .env_remove("MX_KV_SCHEMA")
        .env_remove("MX_KV_DATA")
        .env_remove("MX_DOORS_NAMES")
        .env_remove("MX_DOORS_BUDGET")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn mx");
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to run mx")
}

fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    run(dir, args, "", &[])
}

fn ok(out: &std::process::Output, what: &str) -> String {
    assert!(
        out.status.success(),
        "{what} failed ({:?}): {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn data_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("kv").join("data").join("test.json")
}

fn data(dir: &TempDir) -> serde_json::Value {
    std::fs::read_to_string(data_path(dir))
        .map(|s| serde_json::from_str(&s).unwrap())
        .unwrap_or(serde_json::Value::Null)
}

fn rows(dir: &TempDir, key: &str) -> Vec<serde_json::Value> {
    data(dir)[key]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn facts(dir: &TempDir) -> Vec<serde_json::Value> {
    data(dir)["facts"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Push a fact door and return its kv id.
fn push(dir: &TempDir, value: &str, extra: &[&str]) -> String {
    let mut args = vec!["kv", "push", "facts", value];
    args.extend_from_slice(extra);
    let out = ok(&mx(dir, &args), "kv push");
    out.split_whitespace()
        .next()
        .unwrap()
        .trim_start_matches("kv-")
        .to_string()
}

fn with_paste(typed: &str, body: &str) -> String {
    format!("{typed}\n\n<pasted_content id=\"ab12\">\n{body}\n</pasted_content id=\"ab12\">")
}

fn hook(dir: &TempDir, session: &str, prompt: &str, extra: &[&str]) -> String {
    let input = serde_json::json!({"session_id": session, "prompt": prompt}).to_string();
    let mut args = vec!["doors", "hook"];
    args.extend_from_slice(extra);
    ok(&run(dir, &args, &input, &[]), "doors hook")
}

fn set_mode(dir: &TempDir, session: &str, mode: &str) {
    ok(
        &mx(dir, &["doors", "mode", mode, "--session", session]),
        "doors mode",
    );
}

fn hint_lines(out: &str) -> Vec<&str> {
    out.lines().filter(|l| l.starts_with("[doors] ")).collect()
}

// ---- scope ----

#[test]
fn scope_accepts_the_documented_forms_and_dedups_in_order() {
    let dir = setup();
    push(
        &dir,
        "A pond.",
        &[
            "--trigger",
            "fair haven",
            "--scope",
            "book:walden@3",
            "--scope",
            "house",
            "--scope",
            "book:walden@3",
            "--scope",
            "book:prince",
        ],
    );
    assert_eq!(
        facts(&dir)[0]["scope"],
        serde_json::json!(["book:walden@3", "house", "book:prince"])
    );
}

#[test]
fn scope_rejects_bad_values_and_writes_nothing() {
    let dir = setup();
    for bad in [
        "person:someone",
        "agent:helper",
        "everyone",
        "book:Walden",
        "book:",
        "book:x@0",
        "book:x@",
        "book:x@abc",
        "book:x@01",
        "book:x@99999999999999999999",
    ] {
        let out = mx(
            &dir,
            &[
                "kv",
                "push",
                "facts",
                "v",
                "--trigger",
                "widget",
                "--scope",
                bad,
            ],
        );
        assert_eq!(out.status.code(), Some(4), "{bad}: {}", stderr(&out));
        assert!(
            stderr(&out).contains(&format!("{bad:?}")),
            "{bad}: {}",
            stderr(&out)
        );
        if bad.starts_with("person:") || bad.starts_with("agent:") {
            assert!(
                stderr(&out).contains("later phase (P3)"),
                "{}",
                stderr(&out)
            );
        }
    }
    assert!(facts(&dir).is_empty(), "nothing written");

    let id = push(&dir, "v", &["--trigger", "widget"]);
    let out = mx(
        &dir,
        &[
            "kv",
            "update",
            "facts",
            "--id",
            &format!("kv-{id}"),
            "--scope",
            "agent:x",
        ],
    );
    assert_eq!(out.status.code(), Some(4));
    assert!(facts(&dir)[0].get("scope").is_none());
}

#[test]
fn pinned_warns_once_and_still_succeeds() {
    let dir = setup();
    let out = mx(
        &dir,
        &[
            "kv",
            "push",
            "facts",
            "v",
            "--trigger",
            "widget",
            "--scope",
            "pinned",
        ],
    );
    ok(&out, "push pinned");
    let warn: Vec<String> = stderr(&out)
        .lines()
        .filter(|l| l.contains("pinned fires in every place"))
        .map(str::to_string)
        .collect();
    assert_eq!(warn.len(), 1, "{}", stderr(&out));
}

#[test]
fn update_scope_replaces_clears_and_stands_alone() {
    let dir = setup();
    let id = push(&dir, "v", &["--trigger", "widget", "--scope", "house"]);
    let hash = format!("kv-{id}");
    ok(
        &mx(
            &dir,
            &[
                "kv",
                "update",
                "facts",
                "--id",
                &hash,
                "--scope",
                "book:liberty",
                "--scope",
                "pinned",
            ],
        ),
        "update --scope",
    );
    assert_eq!(
        facts(&dir)[0]["scope"],
        serde_json::json!(["book:liberty", "pinned"])
    );
    assert_eq!(
        facts(&dir)[0]["value"],
        "v",
        "--scope alone leaves the value"
    );

    ok(
        &mx(
            &dir,
            &["kv", "update", "facts", "--id", &hash, "--scope", ""],
        ),
        "update --scope \"\"",
    );
    assert!(facts(&dir)[0].get("scope").is_none(), "cleared, key gone");
    let raw = std::fs::read_to_string(data_path(&dir)).unwrap();
    assert!(!raw.contains("\"scope\""), "absent serializes as before");
}

#[test]
fn kv_triggers_shows_and_filters_scope() {
    let dir = setup();
    let plain = push(&dir, "plain", &["--trigger", "widget"]);
    let book = push(
        &dir,
        "booked",
        &[
            "--trigger",
            "gizmo",
            "--scope",
            "book:walden@3",
            "--scope",
            "pinned",
        ],
    );
    let out = ok(&mx(&dir, &["kv", "triggers"]), "kv triggers");
    assert!(
        out.contains(&format!("  kv-{plain}  [widget]  fires=0\n")),
        "unscoped line unchanged:\n{out}"
    );
    assert!(
        out.contains(&format!(
            "  kv-{book}  [gizmo]  fires=0  [scope: book:walden@3, pinned]\n"
        )),
        "{out}"
    );

    let house = ok(&mx(&dir, &["kv", "triggers", "--scope", "house"]), "filter");
    assert!(house.contains(&plain) && !house.contains(&book), "{house}");
    let pinned = ok(
        &mx(&dir, &["kv", "triggers", "--scope", "pinned"]),
        "filter",
    );
    assert!(
        !pinned.contains(&plain) && pinned.contains(&book),
        "{pinned}"
    );

    let at5 = push(
        &dir,
        "at5",
        &["--trigger", "sprocket", "--scope", "book:walden@5"],
    );
    let bare = push(
        &dir,
        "bare",
        &["--trigger", "cog", "--scope", "book:walden"],
    );
    let other = push(
        &dir,
        "other",
        &["--trigger", "pin", "--scope", "book:prince@3"],
    );
    let filter = |s: &str| ok(&mx(&dir, &["kv", "triggers", "--scope", s]), "filter");
    let walden = filter("book:walden");
    for id in [&book, &at5, &bare] {
        assert!(walden.contains(id.as_str()), "{walden}");
    }
    assert!(
        !walden.contains(&plain) && !walden.contains(&other),
        "{walden}"
    );
    let at3 = filter("book:walden@3");
    assert!(at3.contains(&book), "{at3}");
    for id in [&at5, &bare, &other, &plain] {
        assert!(!at3.contains(id.as_str()), "exact: {at3}");
    }
    assert!(
        !filter("book:wal").contains(&book),
        "a slug prefix is another book"
    );

    let out = mx(&dir, &["kv", "triggers", "--scope", "banana"]);
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    assert!(stderr(&out).contains("\"banana\""), "{}", stderr(&out));
    let out = mx(&dir, &["kv", "triggers", "--scope", "book:x@01"]);
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    let out = mx(&dir, &["kv", "triggers", "--scope", "book:liberty"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(
        stderr(&out).trim(),
        "no triggered entries match --scope book:liberty"
    );

    let json: Vec<serde_json::Value> =
        serde_json::from_str(&ok(&mx(&dir, &["kv", "triggers", "--json"]), "json")).unwrap();
    let by_id = |id: &str| json.iter().find(|r| r["id"] == id).unwrap().clone();
    assert!(by_id(&plain).get("scope").is_none());
    assert_eq!(
        by_id(&book)["scope"],
        serde_json::json!(["book:walden@3", "pinned"])
    );
}

// ---- mx doors mode ----

#[test]
fn mode_needs_a_session_and_good_syntax() {
    let dir = setup();
    let out = mx(&dir, &["doors", "mode", "house"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no session"), "{}", stderr(&out));
    let out = run(
        &dir,
        &["doors", "mode"],
        "",
        &[("CLAUDE_CODE_SESSION_ID", "")],
    );
    assert_eq!(out.status.code(), Some(1));

    for bad in ["book", "book:Walden", "book:x@0", "book:x@01", "pinned"] {
        let out = mx(&dir, &["doors", "mode", bad, "--session", "s"]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
    }
    assert!(rows(&dir, "doors_session").is_empty());
}

#[test]
fn mode_reads_writes_and_rewrites() {
    let dir = setup();
    let read = |dir: &TempDir| ok(&mx(dir, &["doors", "mode", "--session", "s1"]), "read");
    assert_eq!(read(&dir), "house, never set\n");

    let out = ok(
        &run(
            &dir,
            &["doors", "mode", "book:walden@3"],
            "",
            &[("CLAUDE_CODE_SESSION_ID", "s1")],
        ),
        "set",
    );
    assert_eq!(out, "book:walden@3\n");
    let now = read(&dir);
    assert!(now.starts_with("book:walden@3, since "), "{now}");
    assert!(now.ends_with(", set by test\n"), "{now}");

    // Same mode again is a fresh row (a fresh epoch).
    set_mode(&dir, "s1", "book:walden@3");
    let r = rows(&dir, "doors_session");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0]["value"], "mode");
    assert_eq!(
        r[0]["data"],
        serde_json::json!({"session": "s1", "event": "mode", "mode": "book:walden@3", "by": "test"})
    );
    // Another session is untouched.
    assert_eq!(
        ok(&mx(&dir, &["doors", "mode", "--session", "s2"]), "read s2"),
        "house, never set\n"
    );
}

#[test]
fn mode_without_the_schema_key_exits_1_and_prints_the_toml() {
    let dir = setup_schema(&[FACTS_AND_FIRED]);
    let out = mx(&dir, &["doors", "mode", "book:walden", "--session", "s"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("[keys.doors_session]"), "{err}");
    assert!(err.contains("[keys.doors_fired.data.place]"), "{err}");
    // The printed TOML is a working schema addition.
    let toml = &err[err.find("[keys.doors_session]").unwrap()..];
    let dir2 = setup_schema(&[FACTS_AND_FIRED, toml]);
    ok(
        &mx(&dir2, &["doors", "mode", "book:walden", "--session", "s"]),
        "mode with the printed schema",
    );
}

// ---- routing through the hook ----

#[test]
fn book_mode_fires_a_book_door_on_the_paste_and_records_its_place() {
    let dir = setup();
    let wal = push(
        &dir,
        "A pond.",
        &["--trigger", "fair haven", "--scope", "book:walden"],
    );
    let prompt = with_paste("look", WALDEN);
    assert!(
        hook(&dir, "s1", &prompt, &[])
            .lines()
            .all(|l| !l.contains("fair haven"))
    );

    set_mode(&dir, "s1", "book:walden");
    let out = hook(&dir, "s1", &prompt, &[]);
    assert!(
        out.starts_with("\u{1f6aa} fair haven \u{2192} A pond."),
        "{out}"
    );
    let fired = rows(&dir, "doors_fired");
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0]["data"]["entry"], wal.as_str());
    assert_eq!(fired[0]["data"]["place"], "book:walden");
    // The mode row is under an hour old: no foreign_paste row yet.
    let s = rows(&dir, "doors_session");
    assert_eq!(s[0]["value"], "mode");
}

#[test]
fn foreign_paste_row_is_written_at_most_once_an_hour() {
    let dir = setup();
    set_mode(&dir, "s1", "book:walden");
    let prompt = with_paste("look", WALDEN);
    let session_rows = |dir: &TempDir| rows(dir, "doors_session");

    hook(&dir, "s1", &prompt, &[]);
    assert_eq!(session_rows(&dir).len(), 1, "mode row just now: skipped");

    backdate_latest_session_row(&dir, chrono::Duration::minutes(59));
    hook(&dir, "s1", &prompt, &[]);
    assert_eq!(session_rows(&dir).len(), 1, "59 minutes: skipped");

    backdate_latest_session_row(&dir, chrono::Duration::minutes(61));
    hook(&dir, "s1", &prompt, &[]);
    let s = session_rows(&dir);
    assert_eq!(s.len(), 2, "an hour: written");
    assert_eq!(s[0]["value"], "foreign_paste");
    assert_eq!(
        s[0]["data"],
        serde_json::json!({"session": "s1", "event": "foreign_paste"})
    );

    hook(&dir, "s1", &prompt, &[]);
    assert_eq!(session_rows(&dir).len(), 2, "the new row restarts the hour");
}

#[test]
fn place_is_written_only_when_the_schema_declares_it() {
    let dir = setup_schema(&[FACTS_AND_FIRED]);
    push(&dir, "v", &["--trigger", "widget"]);
    hook(&dir, "s1", "widget", &[]);
    let fired = rows(&dir, "doors_fired");
    assert_eq!(fired.len(), 1, "no doors_session, still fires as today");
    assert!(fired[0]["data"].get("place").is_none());

    let dir = setup();
    push(&dir, "v", &["--trigger", "widget", "--scope", "pinned"]);
    hook(&dir, "s1", "widget", &[]);
    assert_eq!(rows(&dir, "doors_fired")[0]["data"]["place"], "pinned");
}

// ---- hints ----

#[test]
fn foreign_hint_prints_once_per_epoch_even_with_no_door() {
    let dir = setup();
    let prompt = with_paste("look", WALDEN);
    let out = hook(&dir, "s1", &prompt, &[]);
    let h = hint_lines(&out);
    assert_eq!(h.len(), 1, "{out}");
    assert!(
        h[0].starts_with("[doors] paste reads as someone else's prose ("),
        "{out}"
    );
    assert!(
        h[0].ends_with("): house doors held on it. Reading? mx doors mode book:<name>"),
        "{out}"
    );
    let s = rows(&dir, "doors_session");
    assert_eq!(
        s[0]["data"],
        serde_json::json!({"session": "s1", "event": "hint", "kind": "foreign", "mode": "house"})
    );

    assert!(
        hint_lines(&hook(&dir, "s1", &prompt, &[])).is_empty(),
        "once"
    );
    assert_eq!(
        hint_lines(&hook(&dir, "s2", &prompt, &[])).len(),
        1,
        "per session"
    );

    set_mode(&dir, "s1", "house");
    assert_eq!(
        hint_lines(&hook(&dir, "s1", &prompt, &[])).len(),
        1,
        "re-armed"
    );
}

#[test]
fn hints_print_after_the_doors() {
    let dir = setup();
    push(&dir, "A widget.", &["--trigger", "widget"]);
    let out = hook(&dir, "s1", &with_paste("widget", WALDEN), &[]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with("\u{1f6aa} widget"));
    assert!(lines[1].starts_with("[doors] "));
}

#[test]
fn ours_hint_in_book_mode() {
    let dir = setup();
    set_mode(&dir, "s1", "book:walden@2");
    let prompt = with_paste("mine", CV);
    let out = hook(&dir, "s1", &prompt, &[]);
    assert_eq!(
        hint_lines(&out),
        vec![
            "[doors] mode is book:walden but this paste reads as ours; routed to house. Done reading? mx doors mode house"
        ]
    );
    assert!(hint_lines(&hook(&dir, "s1", &prompt, &[])).is_empty());
}

#[test]
fn ours_mode_prints_no_hints() {
    let dir = setup();
    set_mode(&dir, "s1", "ours");
    assert!(hint_lines(&hook(&dir, "s1", &with_paste("x", WALDEN), &[])).is_empty());
}

/// Push the latest session row `age` into the past by rewriting its ts in the
/// data file.
fn backdate_latest_session_row(dir: &TempDir, age: chrono::Duration) -> String {
    let mut d = data(dir);
    let ts = (chrono::Utc::now() - age).to_rfc3339();
    d["doors_session"]["entries"][0]["ts"] = ts.clone().into();
    std::fs::write(data_path(dir), serde_json::to_string(&d).unwrap()).unwrap();
    ts
}

#[test]
fn stale_hint_after_six_quiet_hours_with_a_paste() {
    let dir = setup();
    set_mode(&dir, "s1", "book:prince");
    let ts = backdate_latest_session_row(&dir, chrono::Duration::hours(7));
    let hhmm = chrono::DateTime::parse_from_rfc3339(&ts)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string();

    assert!(
        hint_lines(&hook(&dir, "s1", "typed only", &[])).is_empty(),
        "needs a paste"
    );
    let out = hook(&dir, "s1", &with_paste("x", CV), &[]);
    assert!(
        hint_lines(&out).contains(
            &format!("[doors] book:prince since {hhmm}; still reading? mx doors mode house")
                .as_str()
        ),
        "{out}"
    );
    assert!(
        !hint_lines(&hook(&dir, "s1", &with_paste("x", CV), &[]))
            .iter()
            .any(|l| l.contains("still reading")),
        "once per epoch"
    );

    // A fresh mode row resets the timer.
    let dir = setup();
    set_mode(&dir, "s1", "book:prince");
    assert!(
        hint_lines(&hook(&dir, "s1", &with_paste("x", CV), &[]))
            .iter()
            .all(|l| !l.contains("still reading"))
    );
}

#[test]
fn a_hint_that_cannot_be_recorded_is_not_printed() {
    // doors_session without `kind`: mode rows store, hint rows are rejected.
    let dir = setup_schema(&[FACTS_AND_FIRED, PLACE_FIELD, SESSION_KEY]);
    push(&dir, "v", &["--trigger", "widget"]);
    let input =
        serde_json::json!({"session_id": "s1", "prompt": with_paste("widget", WALDEN)}).to_string();
    let out = run(&dir, &["doors", "hook"], &input, &[]);
    let stdout = ok(&out, "hook");
    assert!(hint_lines(&stdout).is_empty(), "{stdout}");
    assert!(
        stdout.starts_with("\u{1f6aa} widget"),
        "doors still open: {stdout}"
    );
    assert!(
        stderr(&out).contains("could not record hint"),
        "{}",
        stderr(&out)
    );
    assert_eq!(rows(&dir, "doors_fired").len(), 1);
}

#[test]
fn no_session_key_means_no_hints_and_no_session_rows() {
    let dir = setup_schema(&[FACTS_AND_FIRED]);
    let out = hook(&dir, "s1", &with_paste("x", WALDEN), &[]);
    assert!(out.is_empty(), "{out}");
    assert!(data(&dir).get("doors_session").is_none());
}

#[test]
fn dry_run_prints_hints_and_writes_nothing() {
    let dir = setup();
    set_mode(&dir, "s1", "book:walden");
    push(
        &dir,
        "A pond.",
        &["--trigger", "fair haven", "--scope", "book:walden"],
    );
    backdate_latest_session_row(&dir, chrono::Duration::hours(8));
    let before = std::fs::read_to_string(data_path(&dir)).unwrap();
    let out = hook(&dir, "s1", &with_paste("x", WALDEN), &["--dry-run"]);
    assert!(out.starts_with("\u{1f6aa} fair haven"), "{out}");
    assert_eq!(hint_lines(&out).len(), 1, "{out}");
    assert_eq!(std::fs::read_to_string(data_path(&dir)).unwrap(), before);
}

// ---- MX_DOORS_BUDGET ----

#[test]
fn budget_flag_beats_env_beats_default() {
    let dir = setup_schema(&[FACTS_AND_FIRED]);
    for n in 0..4 {
        push(&dir, &format!("door {n}"), &["--trigger", "widget"]);
    }
    let input = serde_json::json!({"session_id": "s", "prompt": "widget"}).to_string();
    let count = |args: &[&str], env: &[(&str, &str)]| {
        let mut a = vec!["doors", "hook", "--dry-run"];
        a.extend_from_slice(args);
        ok(&run(&dir, &a, &input, env), "hook").lines().count()
    };
    assert_eq!(count(&[], &[]), 2, "default");
    assert_eq!(count(&[], &[("MX_DOORS_BUDGET", "3")]), 3, "env");
    assert_eq!(
        count(&[], &[("MX_DOORS_BUDGET", "0")]),
        0,
        "0 is the kill switch"
    );
    assert_eq!(
        count(&["--budget", "1"], &[("MX_DOORS_BUDGET", "3")]),
        1,
        "flag wins"
    );
    assert_eq!(
        count(&[], &[("MX_DOORS_BUDGET", "lots")]),
        2,
        "garbage is default"
    );
    assert_eq!(
        count(&[], &[("MX_DOORS_BUDGET", "-1")]),
        2,
        "garbage is default"
    );

    let check = |env: &[(&str, &str)]| {
        let out = ok(
            &run(
                &dir,
                &["doors", "check", "widget", "--dry-run", "--json"],
                "",
                env,
            ),
            "check",
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["fired"].as_array().unwrap().len()
    };
    assert_eq!(check(&[("MX_DOORS_BUDGET", "1")]), 1, "check reads it too");
}

// ---- check --mode ----

#[test]
fn check_mode_and_explain_report_the_mode_and_its_source() {
    let dir = setup();
    push(
        &dir,
        "A pond.",
        &["--trigger", "fair haven", "--scope", "book:walden"],
    );
    let paste = dir.path().join("walden.txt");
    std::fs::write(&paste, WALDEN).unwrap();
    let p = paste.to_str().unwrap();
    let explain = |extra: &[&str]| {
        let mut a = vec!["doors", "check", "look", "--paste", p, "--explain"];
        a.extend_from_slice(extra);
        ok(&mx(&dir, &a), "check --explain")
    };

    let out = explain(&[]);
    assert!(out.starts_with("mode house (default)\n"), "{out}");
    assert!(out.contains("HELD"), "{out}");
    assert!(out.lines().any(|l| l == "hint: foreign"), "{out}");

    let out = explain(&["--mode", "book:walden"]);
    assert!(out.starts_with("mode book:walden (--mode)\n"), "{out}");
    assert!(out.contains("places {book:walden}"), "{out}");
    assert!(
        out.contains("FIRED  seg 2  place book:walden  pool book"),
        "{out}"
    );
    assert!(
        rows(&dir, "doors_session").is_empty(),
        "--mode writes no row"
    );
    assert!(
        rows(&dir, "doors_fired").is_empty(),
        "--explain writes no fire"
    );

    set_mode(&dir, "cli", "book:walden");
    assert!(explain(&[]).starts_with("mode book:walden (session)\n"));

    let json: serde_json::Value =
        serde_json::from_str(&explain(&["--json", "--mode", "ours"])).unwrap();
    assert_eq!(json["mode"], "ours");
    assert_eq!(json["mode_source"], "--mode");
    assert_eq!(json["segments"][1]["places"], serde_json::json!(["house"]));
    assert_eq!(json["hints"], serde_json::json!([]));

    for bad in ["book:", "book:walden@01"] {
        let out = mx(&dir, &["doors", "check", "x", "--mode", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
    }
}

#[test]
fn check_without_explain_never_hints() {
    let dir = setup();
    let paste = dir.path().join("walden.txt");
    std::fs::write(&paste, WALDEN).unwrap();
    let out = ok(
        &mx(
            &dir,
            &["doors", "check", "look", "--paste", paste.to_str().unwrap()],
        ),
        "check",
    );
    assert!(out.is_empty(), "{out}");
    assert!(rows(&dir, "doors_session").is_empty());
}
