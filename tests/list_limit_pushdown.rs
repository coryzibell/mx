//! CLI-level integration tests for `mx memory list --limit`'s SQL pushdown:
//! `KnowledgeStore::list_by_category_limited` +
//! `src/helpers.rs`'s `list_pushdown_eligible` / `fetch_by_categories_budgeted`.
//!
//! This function had ZERO coverage before this pushdown existed
//! (`git grep -c list_by_category origin/main -- src/surreal_db/tests.rs`
//! returns nothing), and the two existing checks that looked adjacent were
//! both misleading: `exclusion_runs_before_limit_truncate` (in
//! `src/helpers.rs`) is named like an invariant guard but hand-builds
//! entries and calls `apply_entry_filters` directly -- it never touches a
//! database and pins a function pushdown routes AROUND when eligible. And
//! `tests/read_path_bench.rs`'s `timed_run` checks exit status only while
//! its `median_ms` discards stdout entirely, so a query silently returning
//! zero rows fast would still pass it.
//!
//! Every test here drives the real binary against a real (embedded, but
//! genuine SurrealDB) store, and every assertion checks WHICH rows came
//! back, not just how many -- the exact failure mode named above,
//! and the one a prior PR in this repo actually shipped (an `IN`-over-
//! composite-index bug returning zero rows fast, read as a "win").
//!
//! Fixture strategy: entry ids are `blake3(domain-or-category ':' title)`
//! truncated to 8 hex chars (`KnowledgeEntry::generate_id`, `src/knowledge.rs`)
//! -- deterministic, but not practically predictable from a black-box CLI
//! test without reimplementing the hash. Every test therefore ESTABLISHES the
//! ids it needs by round-tripping through the CLI (`add --json` returns the
//! assigned id; `list --json` reports the natural ascending-id order) rather
//! than assuming a specific hash output. `--category insight,insight`-style
//! duplicate/ordering assertions go one step further and assert
//! their own precondition explicitly (e.g. "category B genuinely has a row
//! sorting below some row in category A") instead of silently passing
//! vacuously when a fixture doesn't happen to exercise the boundary.

use serde_json::Value;
use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");
const AGENT: &str = "pushdown-bench";

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

fn assert_ok(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} must succeed; stderr: {}",
        stderr_of(out)
    );
}

/// Extract the pretty-printed JSON object from a WRITE-path `--json`
/// response. Pre-existing, out-of-scope wart (see
/// `tests/dedup_write_boundary.rs::extract_json`, which this mirrors):
/// `add_one`'s `(embed skipped)` / `(auto-anchor skipped)` notices print to
/// STDOUT, not stderr, before the caller's JSON payload when
/// `--no-embed`/`--no-auto-anchor` are set, so write-path `--json` stdout is
/// not pure JSON.
fn extract_json(text: &str) -> Value {
    let start = text
        .find("{\n")
        .unwrap_or_else(|| panic!("no JSON object found in stdout: {text:?}"));
    serde_json::from_str(text[start..].trim())
        .unwrap_or_else(|e| panic!("failed to parse extracted JSON ({e}): {:?}", &text[start..]))
}

/// Add one entry, return its assigned id (e.g. "kn-abc12345"). `--no-embed
/// --no-auto-anchor` (established convention, see `tests/dedup_write_boundary.rs`):
/// this file cares about which rows `list` returns, not embedding/anchor
/// side effects, and skipping them keeps the suite from paying real ONNX
/// inference cost on every one of the ~40 rows these tests seed.
fn add(dir: &TempDir, category: &str, title: &str) -> String {
    let out = mx(
        dir,
        &[
            "memory",
            "add",
            "--category",
            category,
            "--title",
            title,
            "--content",
            title,
            "--no-embed",
            "--no-auto-anchor",
            "--json",
        ],
    );
    assert_ok(&out, &format!("add '{title}' in {category}"));
    let v = extract_json(&stdout_of(&out));
    v["id"]
        .as_str()
        .expect("add --json must carry id")
        .to_string()
}

fn update_tags(dir: &TempDir, id: &str, tags: &str) {
    let out = mx(dir, &["memory", "update", id, "--tags", tags]);
    assert_ok(&out, &format!("update {id} --tags {tags}"));
}

/// Run `mx memory list <extra args> --json` and parse the result array.
fn list_json(dir: &TempDir, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["memory", "list"];
    args.extend_from_slice(extra);
    args.push("--json");
    let out = mx(dir, &args);
    assert_ok(&out, &format!("list {extra:?}"));
    serde_json::from_str(&stdout_of(&out)).expect("list --json must be valid JSON array")
}

fn ids_of(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|e| e["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// exactness under a client-side filter (`--tags`) with a fixture where
// the first N rows by id WOULD be thinned by a naive pushdown.
// ---------------------------------------------------------------------------
#[test]
fn tags_filter_forces_fallback_and_stays_exact() {
    let dir = TempDir::new().unwrap();

    // 5 untagged rows first, so we learn their NATURAL ascending-id order
    // from the store itself -- not assumed from the hash.
    for i in 0..5 {
        add(&dir, "gotcha", &format!("tags-filter row {i}"));
    }
    let natural: Vec<String> = ids_of(&list_json(&dir, &["--category", "gotcha"]));
    assert_eq!(natural.len(), 5, "fixture must have exactly 5 rows");

    // Tag the LAST two in natural (ascending id) order -- ids[3], ids[4] --
    // with "x". A naive pushdown (SQL LIMIT 2 BEFORE the tag filter runs)
    // would fetch ids[0], ids[1], filter by tag "x" (neither matches), and
    // return zero rows -- silently wrong. The correct behavior (forced
    // full-fetch fallback, `--tags` is client-side-only) returns ids[3..5].
    update_tags(&dir, &natural[3], "x");
    update_tags(&dir, &natural[4], "x");

    let got = ids_of(&list_json(
        &dir,
        &["--category", "gotcha", "--tags", "x", "--limit", "2"],
    ));
    assert_eq!(
        got,
        vec![natural[3].clone(), natural[4].clone()],
        "must return the two TAGGED entries, not a naive-pushdown truncation \
         of the first two by id (which would come back empty after filtering)"
    );
}

// ---------------------------------------------------------------------------
// `--tags ""` returns zero rows, not N rows.
// ---------------------------------------------------------------------------
#[test]
fn empty_tags_value_returns_zero_rows() {
    let dir = TempDir::new().unwrap();
    for i in 0..3 {
        add(&dir, "reference", &format!("empty-tags row {i}"));
    }
    let got = list_json(
        &dir,
        &["--category", "reference", "--tags", "", "--limit", "5"],
    );
    assert!(
        got.is_empty(),
        "`--tags \"\"` parses to Some([\"\"]), matching no entry's tags -- must \
         return zero rows, not the whole (or limited) table: got {got:?}"
    );
}

// ---------------------------------------------------------------------------
// `--exclude-tags " , "` is pushdown-eligible (parses to an empty
// prefix list) and matches the no-flag result exactly.
// ---------------------------------------------------------------------------
#[test]
fn whitespace_exclude_tags_matches_no_flag_result() {
    let dir = TempDir::new().unwrap();
    for i in 0..4 {
        add(&dir, "decision", &format!("exclude-tags row {i}"));
    }
    let baseline = ids_of(&list_json(
        &dir,
        &["--category", "decision", "--limit", "3"],
    ));
    let with_blank_exclude = ids_of(&list_json(
        &dir,
        &[
            "--category",
            "decision",
            "--exclude-tags",
            " , ",
            "--limit",
            "3",
        ],
    ));
    assert_eq!(
        baseline, with_blank_exclude,
        "`--exclude-tags \" , \"` parses to zero prefixes and must behave \
         identically to no flag at all -- checking the raw Option<String> \
         instead of the parsed list would wrongly force the (still-correct, \
         but here unnecessary) fallback path"
    );
    assert_eq!(baseline.len(), 3);
}

// ---------------------------------------------------------------------------
// ordering identity: user-typed category order, and the bare-`list`
// alphabetical-category-then-id order. Explicitly asserts its own crossing
// precondition so it cannot pass vacuously (a global-LIMIT bug can only be
// caught if some row in the second category sorts below some row in the
// first).
// ---------------------------------------------------------------------------
#[test]
fn user_typed_category_order_is_preserved_under_limit() {
    let dir = TempDir::new().unwrap();
    // "technique" typed BEFORE "bloom" -- the reverse of alphabetical.
    for i in 0..3 {
        add(&dir, "technique", &format!("typed-order technique {i}"));
    }
    for i in 0..3 {
        add(&dir, "bloom", &format!("typed-order bloom {i}"));
    }
    let technique_ids = ids_of(&list_json(&dir, &["--category", "technique"]));
    let bloom_ids = ids_of(&list_json(&dir, &["--category", "bloom"]));

    // Precondition: at least one bloom id sorts below at least one technique
    // id, or a global-LIMIT (instead of per-category-budgeted) bug could
    // never be distinguished from a correct implementation here.
    let min_bloom = bloom_ids.iter().min().unwrap();
    let max_technique = technique_ids.iter().max().unwrap();
    assert!(
        min_bloom < max_technique,
        "fixture precondition failed: no bloom id ({bloom_ids:?}) sorts below \
         any technique id ({technique_ids:?}) -- this test cannot distinguish \
         user-typed order from a global id sort and would pass vacuously; \
         widen the fixture"
    );

    let expected: Vec<String> = technique_ids
        .iter()
        .chain(bloom_ids.iter())
        .take(4)
        .cloned()
        .collect();
    let got = ids_of(&list_json(
        &dir,
        &["--category", "technique,bloom", "--limit", "4"],
    ));
    assert_eq!(
        got, expected,
        "`--category technique,bloom` must walk categories in TYPED order \
         (all of technique's budget first, then bloom's), never re-sorted by \
         a global id order"
    );
}

#[test]
fn bare_list_uses_alphabetical_category_order() {
    let dir = TempDir::new().unwrap();
    // "session" > "insight" alphabetically; typed nowhere (bare `list`), so
    // the order must come from `list_categories()` (alphabetical), not
    // insertion order.
    for i in 0..3 {
        add(&dir, "session", &format!("alpha-order session {i}"));
    }
    for i in 0..3 {
        add(&dir, "insight", &format!("alpha-order insight {i}"));
    }
    let session_ids = ids_of(&list_json(&dir, &["--category", "session"]));
    let insight_ids = ids_of(&list_json(&dir, &["--category", "insight"]));

    let min_session = session_ids.iter().min().unwrap();
    let max_insight = insight_ids.iter().max().unwrap();
    assert!(
        min_session < max_insight,
        "fixture precondition failed: no session id sorts below any insight \
         id -- widen the fixture so category order is actually distinguishable \
         from a global id sort"
    );

    // Bare `list --limit N` walks ALL 8 categories alphabetically:
    // bloom, decision, gotcha, insight, pattern, reference, session, technique.
    // Only insight/session carry rows here, so the expected prefix is
    // insight's rows (in full, since 3 < limit) followed by session's.
    let expected: Vec<String> = insight_ids
        .iter()
        .chain(session_ids.iter())
        .take(4)
        .cloned()
        .collect();
    let got = ids_of(&list_json(&dir, &["--limit", "4"]));
    assert_eq!(
        got, expected,
        "bare `list --limit` must walk categories in list_categories() \
         (alphabetical) order: insight before session"
    );
}

// ---------------------------------------------------------------------------
// budget spanning a category boundary.
// ---------------------------------------------------------------------------
#[test]
fn budget_spans_category_boundary() {
    let dir = TempDir::new().unwrap();
    for i in 0..2 {
        add(&dir, "pattern", &format!("budget-boundary pattern {i}"));
    }
    for i in 0..4 {
        add(&dir, "gotcha", &format!("budget-boundary gotcha {i}"));
    }
    let pattern_ids = ids_of(&list_json(&dir, &["--category", "pattern"]));
    let gotcha_ids = ids_of(&list_json(&dir, &["--category", "gotcha"]));
    assert_eq!(pattern_ids.len(), 2);
    assert_eq!(gotcha_ids.len(), 4);

    // |pattern| = 2 < limit = 5: expect all of pattern, then the first 3 of
    // gotcha, in order.
    let expected: Vec<String> = pattern_ids
        .iter()
        .chain(gotcha_ids.iter().take(3))
        .cloned()
        .collect();
    let got = ids_of(&list_json(
        &dir,
        &["--category", "pattern,gotcha", "--limit", "5"],
    ));
    assert_eq!(got, expected);
}

// ---------------------------------------------------------------------------
// `--category insight,insight --limit 5` against a 3-row category
// returns 5 rows WITH duplicates (never deduped).
// ---------------------------------------------------------------------------
#[test]
fn repeated_category_reproduces_duplicates() {
    let dir = TempDir::new().unwrap();
    for i in 0..3 {
        add(&dir, "insight", &format!("dup-category row {i}"));
    }
    let natural = ids_of(&list_json(&dir, &["--category", "insight"]));
    assert_eq!(natural.len(), 3);

    let got = ids_of(&list_json(
        &dir,
        &["--category", "insight,insight", "--limit", "5"],
    ));
    // First 3 = the whole category (first pass), then the budget carries
    // over into the SECOND "insight" and re-fetches from the start: 2 more
    // rows, duplicating natural[0] and natural[1].
    let mut expected = natural.clone();
    expected.push(natural[0].clone());
    expected.push(natural[1].clone());
    assert_eq!(
        got, expected,
        "--category insight,insight must reproduce duplicates across the \
         repeated category, never dedupe the category list"
    );
}

// ---------------------------------------------------------------------------
// --limit greater than the table, and --limit 0.
// ---------------------------------------------------------------------------
#[test]
fn limit_larger_than_table_returns_everything() {
    let dir = TempDir::new().unwrap();
    for i in 0..3 {
        add(&dir, "bloom", &format!("limit-over row {i}"));
    }
    let natural = ids_of(&list_json(&dir, &["--category", "bloom"]));
    let got = ids_of(&list_json(&dir, &["--category", "bloom", "--limit", "999"]));
    assert_eq!(got, natural);
}

#[test]
fn limit_zero_returns_nothing() {
    let dir = TempDir::new().unwrap();
    for i in 0..3 {
        add(&dir, "bloom", &format!("limit-zero row {i}"));
    }
    let got = list_json(&dir, &["--category", "bloom", "--limit", "0"]);
    assert!(
        got.is_empty(),
        "--limit 0 must return zero rows, got {got:?}"
    );
}
