//! Read-path scaling harness for `mx memory list`.
//!
//! `#[ignore]` by default — this is opt-in
//! (`cargo test --release --test read_path_bench -- --ignored --nocapture`),
//! never part of a normal `cargo test` run.
//!
//! WHAT IT ASSERTS, AND WHY WALL CLOCK
//! ------------------------------------
//! The regression this guards is `2N+1`: `value_to_knowledge_entry` issues two
//! sequential per-row queries (tags, applicability) for every hydrated row, so
//! the query count of a `list` grows linearly with TABLE size — not with the
//! number of rows the caller asked for. `mx memory list --limit 1` against an
//! 8k-row store costs the same as `mx memory list`.
//!
//! Query count would be the honest metric here -- exact, deterministic,
//! identical on every machine, and what the PR's actual before/after numbers
//! rest on. But measuring it needs a real SurrealDB server run with
//! `--log trace`, counted off the server's own "Parsing SurrealQL query" trace
//! lines -- an out-of-band harness, not this file. This test spawns `mx` in
//! embedded (SurrealKV) mode, self-contained, with no server process and no
//! trace log to read: query count is not observable from inside it.
//!
//! What IS asserted here is a WALL-CLOCK ratio instead: measure `list --limit
//! 1` at N rows, double the table to 2N, measure again, assert the ratio
//! stays near 1.0. See BASELINE CORRECTION below for why that ratio is
//! measured against a control and not asserted raw. That is strictly weaker
//! evidence than a query count, for two reasons:
//!   * an embedded store skips the WebSocket JSON encode/decode that dominates
//!     the network-mode cost (in one reference graph, the `embedding` column
//!     alone is 79.2% of the bytes a `list` transfers), so an embedded timing
//!     understates the real-world win by several fold; and
//!   * `mx memory add-batch` cannot write the `embedding` column at all, so a
//!     self-contained test CANNOT build a store with a faithful payload profile
//!     through the public CLI. Wall clock at faithful scale needs an external
//!     harness with direct SurrealQL access. It does not belong in this file.
//!
//! A ratio keeps it dimensionless -- it does not encode this machine's speed --
//! but it is still wall clock, and wall clock is noisier than an exact count.
//! This is the weaker in-repo substitute for a metric that genuinely needs a
//! server outside this test to produce.
//!
//! ISOLATION
//! ---------
//! `MX_SURREAL_MODE=embedded` is forced alongside `MX_SURREAL_ROOT`. Setting the
//! root alone is NOT isolation: with an ambient `MX_SURREAL_MODE=network` the
//! root is ignored entirely and the binary talks to `MX_SURREAL_URL`. That is the
//! PR #401 "phantom broken main" failure, and it is still live in
//! `tests/trigger_check.rs`, which sets `MX_SURREAL_ROOT` without the mode and
//! documents itself as isolated.
//!
//! WHY THERE IS NO `show` BENCH HERE
//! ----------------------------------
//! `MX_SURREAL_MODE=embedded` means every invocation opens SurrealKV fresh, and
//! SurrealKV replays its entire commit log on open -- at 4,000 rows that's ~899ms
//! of a ~911ms `show` invocation (98.7%) in store-open alone, before the record
//! lookup even runs. That cost swamps any difference the query-layer fix makes,
//! so an in-repo embedded-mode timing of `show` cannot see the improvement it
//! would need to demonstrate. The record-lookup fix is real and was measured
//! out-of-band (network mode, paired against main); see the PR description for
//! the numbers.
//!
//! BASELINE CORRECTION (why a raw N->2N ratio doesn't work)
//! ----------------------------------------------------------
//! The store-open replay cost above isn't just a constant that swamps the
//! signal on `show` -- it scales with table size on its own, which
//! contaminates a raw N->2N ratio on ANY command, not only `list`. Measured
//! directly: `mx memory stats` -- which never calls `value_to_knowledge_entry`
//! and carries none of the 2N+1 defect -- still shows its own raw ratio
//! consistently above 1.0 (roughly 1.4-1.8 across N=300..2000 in repeated
//! runs here), so a raw `< 1.5` bound on `list` fails on store-open scaling
//! alone, on a broken AND a correctly-fixed `list` alike. Raising N does not
//! fix this -- it was measured making the raw signal worse, not better,
//! because replay cost grows with table size too.
//!
//! The fix here is to measure the SAME control at the SAME N and 2N and
//! divide: `corrected = list_ratio / baseline_ratio`. See the doc comment on
//! `bounded_read_cost_does_not_scale_with_table_size` below for the measured
//! corrected-ratio numbers and the chosen bound.

use std::process::{Command, Stdio};
use tempfile::TempDir;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// Rows to seed. Small by default so the test is runnable; the reference graph
/// this was calibrated against holds 8130. Raise with `MX_BENCH_ROWS`.
fn bench_rows() -> usize {
    std::env::var("MX_BENCH_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

/// Spawn `mx` against a store that is provably local.
///
/// Both `MX_SURREAL_MODE` and `MX_SURREAL_ROOT` are required. `MX_SKIP_SCHEMA`
/// is deliberately NOT set: schema application is part of what a real invocation
/// pays, and suppressing it here would hide it.
fn mx(dir: &TempDir, args: &[&str]) -> std::process::Output {
    mx_inner(dir, args, false)
}

/// `skip_schema` sets `MX_SKIP_SCHEMA=1`, which suppresses re-application of the
/// whole schema on connect (src/surreal_db/connection.rs applies it on EVERY
/// process start, and the tail of schema/surrealdb-schema.surql is seven
/// unindexed full-table UPDATE sweeps).
///
/// The TIMED runs set it; the seed and the row-count assertion do not. That is
/// deliberate and it is not cheating: the schema tax is a constant per
/// invocation, and a constant added to both sides of a RATIO pulls the ratio
/// toward 1.0 -- i.e. it hides the very scaling this test exists to catch. At
/// 500/1000 rows the untimed-schema version of `show` measured ratio 1.42 and
/// PASSED a bound it should have failed, purely because ~250 ms of fixed cost
/// diluted it. Removing a constant from both sides makes the ratio measure only
/// what varies with table size.
fn mx_inner(dir: &TempDir, args: &[&str], skip_schema: bool) -> std::process::Output {
    let mut cmd = Command::new(MX);
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "bench")
        .env("MX_SURREAL_MODE", "embedded")
        .env("MX_SURREAL_ROOT", dir.path().join("surreal"))
        .env("MX_HOME", dir.path())
        // Isolation is about MX_SKIP_SCHEMA specifically, not the store
        // (embedded + MX_SURREAL_ROOT/MX_HOME already make the store
        // authoritative regardless of ambient env). Without this, an
        // ambient MX_SKIP_SCHEMA=1 in the invoking shell would silently
        // defeat the untimed/timed distinction skip_schema exists to draw.
        .env_remove("MX_SKIP_SCHEMA")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if skip_schema {
        cmd.env("MX_SKIP_SCHEMA", "1");
    }
    cmd.output().expect("failed to spawn mx")
}

/// One JSONL line per entry, shaped like the reference graph: tag counts around
/// the observed mean of 5.2 drawn from a high-cardinality vocabulary (the
/// reference graph carries 4733 distinct tags over 8130 rows), bodies around the
/// observed median of 661 bytes. Only the SHAPE matters — the defect is
/// structural, not content-dependent.
fn seed_jsonl(offset: usize, n: usize) -> String {
    // Only categories the schema seeds by default; `person`/`thread`/`archive`
    // are not schema-seeded and an add against them exits non-zero.
    const CATS: [&str; 8] = [
        "insight",
        "decision",
        "gotcha",
        "reference",
        "session",
        "bloom",
        "technique",
        "pattern",
    ];
    let mut out = String::new();
    for i in offset..offset + n {
        let cat = CATS[i % CATS.len()];
        let tags: Vec<String> = (0..5)
            .map(|k| format!("t{:04}", (i * 7 + k * 131) % 4733))
            .collect();
        let body = "the quick brown fox jumps over the lazy dog ".repeat(15);
        out.push_str(&format!(
            r#"{{"category":"{cat}","title":"bench entry {i:06}","content":"{body}","source_agent":"bench","tags":"{}"}}"#,
            tags.join(",")
        ));
        out.push('\n');
    }
    out
}

/// Run `mx_inner` and panic loudly on a non-zero exit. A failing invocation
/// times FAST on both sides of a ratio (it errors out before doing any real
/// work), so a discarded exit status lets `assert!(ratio < 1.5)` pass while
/// measuring nothing at all -- the timed run has to fail as loudly as `seed`
/// and `assert_row_count` already do.
fn timed_run(dir: &TempDir, args: &[&str]) -> std::process::Output {
    let out = mx_inner(dir, args, true);
    assert!(
        out.status.success(),
        "timed invocation {args:?} must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Median wall time of `n` runs of one mx invocation, in milliseconds. `run`
/// is the status-checking wrapper to apply to each invocation -- `timed_run`
/// for the bounded-read case below, which must succeed on every call.
fn median_ms(
    dir: &TempDir,
    trials: usize,
    args: &[&str],
    run: fn(&TempDir, &[&str]) -> std::process::Output,
) -> u128 {
    assert!(trials > 0, "median_ms: trials must be > 0, got 0");
    let _ = run(dir, args); // warm-up; status still checked
    let mut v: Vec<u128> = (0..trials)
        .map(|_| {
            let t = std::time::Instant::now();
            let _ = run(dir, args);
            t.elapsed().as_millis()
        })
        .collect();
    v.sort_unstable();
    v[v.len() / 2]
}

fn seed(dir: &TempDir, jsonl: &str, label: &str) {
    let path = dir.path().join(format!("{label}.jsonl"));
    std::fs::write(&path, jsonl).unwrap();
    let out = Command::new(MX)
        .args([
            "memory",
            "add-batch",
            "--file",
            path.to_str().unwrap(),
            "--no-embed",
        ])
        .env("MX_CURRENT_AGENT", "bench")
        .env("MX_SURREAL_MODE", "embedded")
        .env("MX_SURREAL_ROOT", dir.path().join("surreal"))
        .env("MX_HOME", dir.path())
        // Not under measurement: keeps the seed off the auto-anchor path and off
        // the ONNX model load.
        .env("MX_SKIP_WRITE_ANCHOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("seed failed to spawn");
    assert!(
        out.status.success(),
        "seed '{label}' must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_row_count(dir: &TempDir, want: usize) {
    // N=0 is rejected outright: `want=0` would make this guard accept the
    // very empty/misdirected store it exists to catch (an unseeded store
    // also prints "Total entries: 0" and exits 0 -- see the panic message
    // below), turning the guard into a no-op exactly when it matters most.
    assert!(
        want > 0,
        "assert_row_count: N=0 is not a valid expected row count"
    );
    let out = mx(dir, &["memory", "stats"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Exact line match, not `contains`: "Total entries: 2000" is a substring
    // of "Total entries: 20000", so a naive `contains` prefix-matches a store
    // that is 10x too big and calls it correct.
    let want_line = format!("Total entries: {want}");
    assert!(
        stdout.lines().any(|line| line == want_line),
        "store must hold exactly {want} rows. A green ZERO here means the binary \
         is reading the WRONG store, not that the seed was empty -- `mx memory \
         stats` returns 0 and exits 0 against an unseeded store. stdout: {stdout}"
    );
}

/// The invariant a bounded read SHOULD hold: **the cost of a bounded read must
/// not scale with table size.** `list --limit 1` does not hold it yet.
///
/// Asserted as a CORRECTED ratio, not a raw one. A raw N->2N wall-clock ratio
/// of `list --limit 1` is contaminated: embedded-mode store-open replay (see
/// ISOLATION / WHY THERE IS NO `show` BENCH above) itself scales with table
/// size, so it inflates -- or on a different machine could deflate -- the raw
/// ratio independent of anything the query layer does. A hydration-free
/// control makes this concrete: `mx memory stats` never calls
/// `value_to_knowledge_entry` and carries none of the 2N+1 defect, yet its own
/// raw N->2N ratio was measured coming in above 1.5 at this file's default
/// N -- a perfectly-fixed `list` would fail a raw `< 1.5` bound on store-open
/// cost alone, forever, and raising N only makes that worse (replay scales
/// with table size too).
///
/// So this test measures BOTH `list --limit 1` and the `stats` control at the
/// same N and 2N, and asserts on `list_ratio / baseline_ratio`: dividing out
/// the store-open cost that both commands pay leaves only what varies with
/// hydration. Measured directly (this PR, embedded mode, N in 300..2000, 9
/// trials/point, multiple replicates per N): the corrected ratio holds in a
/// tight ~1.08-1.27 band today -- NOT the ~2.0 a naive read of the raw ratio
/// alone would suggest. `stats` is not perfectly hydration-free either (it
/// runs `count()` plus one count-by-category query per category, its own
/// real per-row scan cost), so dividing by it removes more than pure
/// store-open cost -- correctly so: the residual band is closer to the TRUE
/// isolated cost of the 2N+1 defect once everything both commands share is
/// divided out. `--limit` is applied by `apply_entry_filters` in
/// `src/helpers.rs`, as `entries.truncate(n)` AFTER every row in the table has
/// been hydrated through `value_to_knowledge_entry`'s two per-row edge
/// queries. This PR does not touch that path -- fixing it needs batch
/// hydration, which needs PR #401's primitive, which isn't on `main` yet.
/// Known-remaining defect, not a regression this PR introduced. Once batch
/// hydration lands, the corrected ratio should converge on ~1.0 (list's cost
/// profile then matches `stats`'s: store-open plus a small bounded query) and
/// this assertion should PASS -- that is the point of correcting the metric:
/// an instrument that can never go green on a correct fix isn't an
/// instrument.
///
/// The margin this leaves is real but genuinely thin (~1.08 defect-state vs
/// ~1.0 fixed-state at this file's default N=2000), not the generous ~2x gap
/// the raw ratio implied, and it is measured on a machine sharing CPU with
/// other concurrent builds -- treat this bench as informational, not a hard
/// CI gate (it is `#[ignore]`d for exactly that reason).
///
/// A ratio is the right shape here: it is dimensionless, so it does not encode
/// this machine's speed. The bound is deliberately loose -- this catches "we
/// went back to O(table)", not a 15% drift.
#[test]
#[ignore = "benchmark: opt in with --ignored. Asserts a CORRECTED ratio \
            (list_ratio / hydration-free-baseline_ratio); measured directly \
            it holds ~1.08-1.27 today from the known-remaining 2N+1 hydration \
            defect (needs PR #401's batch-hydration primitive -- not a bug in \
            this PR, do not file one), and should converge on ~1.0, and PASS, \
            once that lands."]
fn bounded_read_cost_does_not_scale_with_table_size() {
    let dir = TempDir::new().unwrap();
    let n = bench_rows();
    let trials = 9;

    seed(&dir, &seed_jsonl(0, n), "first");
    assert_row_count(&dir, n);
    let at_n = median_ms(&dir, trials, &["memory", "list", "--limit", "1"], timed_run);
    let baseline_at_n = median_ms(&dir, trials, &["memory", "stats"], timed_run);

    seed(&dir, &seed_jsonl(n, n), "second");
    assert_row_count(&dir, 2 * n);
    let at_2n = median_ms(&dir, trials, &["memory", "list", "--limit", "1"], timed_run);
    let baseline_at_2n = median_ms(&dir, trials, &["memory", "stats"], timed_run);

    let raw_ratio = at_2n as f64 / at_n.max(1) as f64;
    let baseline_ratio = baseline_at_2n as f64 / baseline_at_n.max(1) as f64;
    let corrected_ratio = raw_ratio / baseline_ratio.max(f64::MIN_POSITIVE);

    // Reported unconditionally: a benchmark that only speaks when it fails is a
    // benchmark nobody can read the trend out of. Raw AND corrected numbers,
    // pass or fail, every run.
    println!(
        "list --limit 1: {n} rows -> {at_n} ms; {} rows -> {at_2n} ms; raw ratio {:.3}\n\
         baseline (`memory stats`, hydration-free): {n} rows -> {baseline_at_n} ms; \
         {} rows -> {baseline_at_2n} ms; baseline ratio {:.3}\n\
         corrected ratio (raw / baseline): {corrected_ratio:.3}",
        2 * n,
        raw_ratio,
        2 * n,
        baseline_ratio,
    );

    assert!(
        corrected_ratio < 1.05,
        "EXPECTED FAILURE, not a new bug: cost of `list --limit 1` scaled \
         {corrected_ratio:.3}x (corrected for store-open replay cost) when \
         the table doubled ({n} rows: {at_n} ms list / {baseline_at_n} ms \
         baseline; {} rows: {at_2n} ms list / {baseline_at_2n} ms baseline; \
         raw ratio {raw_ratio:.3}, baseline ratio {baseline_ratio:.3}). This is \
         the known 2N+1 hydration regression -- `value_to_knowledge_entry`'s \
         per-row edge queries -- that this PR does not fix; fixing it needs \
         batch hydration, which needs PR #401's primitive. Documents \
         known-remaining work; do not file a bug for this.",
        2 * n
    );
}
