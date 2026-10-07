//! Read-path scaling harness for `mx memory list`.
//!
//! `#[ignore]` by default — this is opt-in
//! (`cargo test --release --test read_path_bench -- --ignored --nocapture`),
//! never part of a normal `cargo test` run.
//!
//! HOW THIS FILE CAN GO GREEN WHILE PROVING NOTHING
//! --------------------------------------------------------------------------
//! Four separate ways a run of this file reports success without having
//! measured what it claims to. Known collectively because closing #3
//! surfaced the general shape; listed together here so
//! the next person reaching for "the bench passes" knows what that phrase
//! does and doesn't mean:
//!
//! 1. **Wrong invocation, no benchmark ran at all.** `cargo test --release
//!    --test read_path_bench` (no `-- --ignored`) prints `test result: ok`
//!    and exits 0 having run ZERO tests from this file — every test here is
//!    `#[ignore]`d by design (see below). The flag is required:
//!    `cargo test --release --test read_path_bench -- --ignored --nocapture`.
//!    Citing "the bench passes" from a plain `cargo test` run is citing a
//!    run that never happened.
//! 2. **Exit status only, stdout discarded.** `timed_run` checks the child
//!    process's exit code; `median_ms` discards its stdout entirely to
//!    compute a timing. A `list --limit 1` that silently returns the wrong
//!    row, or zero rows, still exits 0 and still produces a (fast, since it
//!    did less work) timing that can pass the ratio bound.
//! 3. **The seeding-vs-measured-command gap (closed).** `assert_row_count`
//!    guards the SEED at each step; nothing originally checked that the
//!    exact command the timing loop runs (`list --limit 1 --json`) returns
//!    what it claims to. `assert_list_limit_one_returns_one_row` (below,
//!    called right after each `assert_row_count`) closes this specific gap
//!    -- named here as an instance of the same class as #2, not a separate
//!    open item.
//! 4. **`MX_BENCH_ROWS` set to anything but `CALIBRATED_N`.** The bound
//!    assertion is gated on `n == CALIBRATED_N` (see below); any other N
//!    prints its numbers and returns EARLY, past every real assertion,
//!    still reporting `test ... ok`. A green run at an unexamined
//!    `MX_BENCH_ROWS` value asserted nothing.
//!
//! WHAT IT ASSERTS, AND WHY WALL CLOCK
//! ------------------------------------
//! The regression this guards is `2N+1`: `value_to_knowledge_entry` issues two
//! sequential per-row queries (tags, applicability) for every hydrated row, so
//! the query count of a `list` grows linearly with TABLE size — not with the
//! number of rows the caller asked for. That was true of `mx memory list
//! --limit 1` before `--limit` was pushed into the SQL query (budgeted per
//! category) whenever no client-side-only filter is active — the case this
//! bench exercises no longer scales with table size; an unbounded `mx memory
//! list` (no pushdown-eligible `--limit`) still does.
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
//! `MX_SURREAL_MODE=embedded` is forced alongside `MX_SURREAL_ROOT`, via
//! `tests/common::isolate` (see `mx_inner` below). Setting the root alone is
//! NOT isolation: with an ambient `MX_SURREAL_MODE=network` the root is
//! ignored entirely and the binary talks to `MX_SURREAL_URL`. That is the
//! PR #401 "phantom broken main" failure; `common::isolate` is this repo's
//! shared fix for it, now used by this file and five others.
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
//! consistently above 1.0 (roughly 1.3-1.8 across N=300..4000 in repeated
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

/// `timed_run` checks exit status only, and
/// `median_ms` discards stdout entirely -- harmless while this bench was a
/// documented expected-failure, not harmless now that a green run is being
/// cited as evidence the pushdown itself works. `assert_row_count` guards
/// the SEED, not the measured command -- a `list --limit 1` silently
/// returning zero rows fast would still pass every timing assertion below.
/// One untimed, exit-checked call per seed step, right after
/// `assert_row_count`, pins that the exact command the timing loop runs
/// actually returns the row it asks for.
fn assert_list_limit_one_returns_one_row(dir: &TempDir) {
    let out = mx(dir, &["memory", "list", "--limit", "1", "--json"]);
    assert!(
        out.status.success(),
        "untimed `list --limit 1 --json` must succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("list --limit 1 --json must be valid JSON");
    assert_eq!(
        parsed.as_array().unwrap().len(),
        1,
        "list --limit 1 must return exactly one row -- a query returning \
         zero rows fast would still pass every timing assertion in this \
         file; stdout: {stdout}"
    );
}

mod common;

const MX: &str = env!("CARGO_BIN_EXE_mx");

/// The N this file's assertion is calibrated against. Kept at 300
/// unchanged by the LIMIT pushdown fix below -- what changed is why. Before
/// that fix, 300 was chosen because the margin between the DEFECT-STATE
/// ratio (~1.17-1.38) and the `< 1.05` bound cleared the measured
/// between-environment noise floor by 2-3x at this N; that margin decayed
/// monotonically toward 1.0 as N rose (see BASELINE CORRECTION above),
/// shrinking to roughly 1x (a coin flip) by N=2000 and inverting into a
/// false green by N=4000 on an UNFIXED `list`. That defect-state margin no
/// longer exists -- the LIMIT pushdown holds this bench at ~0.88,
/// comfortably under the bound at any N a table this shape would plausibly
/// use. 300 is retained anyway, for two reasons that don't depend on the
/// old margin: it's what keeps the bound able to detect a REGRESSION back
/// to full-table hydration (the failure mode this file exists to catch),
/// and it's what keeps before-and-after numbers
/// comparable across runs of this file. A larger N still compresses the
/// ratio toward 1.0 -- and therefore toward a bound that can no longer tell
/// a working pushdown from a regressed one -- so do not raise it to chase a
/// "more realistic" table size.
const CALIBRATED_N: usize = 300;

/// Rows to seed. Defaults to `CALIBRATED_N`. `MX_BENCH_ROWS` overrides it for
/// exploration, but the assertion below only fires at `CALIBRATED_N` -- see
/// `bounded_read_cost_does_not_scale_with_table_size`.
fn bench_rows() -> usize {
    match std::env::var("MX_BENCH_ROWS") {
        Err(_) => CALIBRATED_N,
        Ok(v) => v.parse().unwrap_or_else(|e| {
            panic!(
                "MX_BENCH_ROWS={v:?} is not a valid usize ({e}); a silent \
                 fallback to the default here would hide a typo'd row count \
                 behind a bench that quietly ran the wrong N"
            )
        }),
    }
}

/// Spawn `mx` against a store that is provably local.
///
/// Isolation is delegated to `common::isolate`: it forces embedded mode with
/// an explicit `MX_SURREAL_ROOT`/`MX_HOME`, and -- unlike this file's old
/// hand-rolled version -- strips every ambient `MX_SURREAL_*` plus
/// `MX_MEMORY_PATH`/`MX_MEMORY_BACKEND` that could otherwise redirect the
/// child at a shared or production store. `MX_SKIP_SCHEMA` is deliberately
/// NOT set here: schema application is part of what a real invocation pays,
/// and suppressing it here would hide it.
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
    common::isolate(&mut cmd, dir.path());
    cmd.args(args)
        .env("MX_CURRENT_AGENT", "bench")
        // Isolation of the STORE is common::isolate's job (above). This is
        // about MX_SKIP_SCHEMA specifically: without stripping it, an
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
    let mut cmd = Command::new(MX);
    common::isolate(&mut cmd, dir.path());
    let out = cmd
        .args([
            "memory",
            "add-batch",
            "--file",
            path.to_str().unwrap(),
            "--no-embed",
        ])
        .env("MX_CURRENT_AGENT", "bench")
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
/// not scale with table size.** `list --limit 1` held it since the LIMIT
/// pushdown (see below) for the no-client-side-filter case this bench
/// exercises.
///
/// Asserted as a CORRECTED ratio, not a raw one. A raw N->2N wall-clock ratio
/// of `list --limit 1` is contaminated: embedded-mode store-open replay (see
/// ISOLATION / WHY THERE IS NO `show` BENCH above) itself scales with table
/// size, so it inflates -- or on a different machine could deflate -- the raw
/// ratio independent of anything the query layer does. A control without
/// per-row hydration makes this concrete: `mx memory stats` never calls
/// `value_to_knowledge_entry` and carries none of the 2N+1 defect, yet its own
/// raw N->2N ratio was measured coming in above 1.5 at this file's default
/// N -- a perfectly-fixed `list` would fail a raw `< 1.5` bound on store-open
/// cost alone, forever, and raising N only makes that worse (replay scales
/// with table size too).
///
/// So this test measures BOTH `list --limit 1` and the `stats` control at the
/// same N and 2N, and asserts on `list_ratio / baseline_ratio`: dividing out
/// the store-open cost that both commands pay leaves only what varies with
/// hydration. `--limit` used to be applied by `apply_entry_filters` in
/// `src/helpers.rs`, as `entries.truncate(n)` AFTER every row in the table
/// had been hydrated through `value_to_knowledge_entry`'s two per-row edge
/// queries -- the 2N+1 defect this bench was written to catch, measured at
/// ~1.17-1.38 corrected ratio (across two environments, N=300, 9
/// trials/point) before the LIMIT pushdown below.
///
/// The LIMIT pushdown (`KnowledgeStore::list_by_category_limited`,
/// `src/helpers.rs`'s `fetch_by_categories_budgeted`) pushes `--limit` into
/// the SQL query itself, budgeted per category, whenever no client-side-only
/// filter is active (`list_pushdown_eligible`) -- which is exactly the case
/// this bench exercises: no `--category`, no `--tags`, nothing
/// `apply_entry_filters` would need to run after hydration. `list --limit 1`
/// now stops fetching once its budget of 1 row is met, in the FIRST category
/// that has any rows, independent of how large the rest of the table is --
/// so this bench's corrected ratio is measured (release, embedded mode,
/// N=300, this file's calibrated default) at ~0.88, comfortably under the
/// `< 1.05` bound below, and PASSES. Batch hydration (PR #401's
/// `get_tags_for_entries_async` / `get_applicability_for_entries_async`,
/// merged, `src/surreal_db/relationships.rs`) was the originally-planned fix
/// for this regression but is NOT what closed it -- the LIMIT pushdown made
/// it moot for the bounded case this bench measures. Batch hydration remains
/// a real, separate win for the UNbounded case (a `list`/`search` with no
/// pushdown-eligible `--limit`, or a `--limit` large enough that per-row
/// hydration cost dominates): measured at 1.79ms/row batched vs 1.70ms/row
/// per-row today, worth ~3.10s -> 37ms at `--limit 20` once wired in, but
/// that is a separate PR this bench does not exercise.
///
/// The margin this leaves is real but was genuinely thin before the LIMIT
/// pushdown (~1.17-1.38 defect-state vs ~1.0 fixed-state at this file's calibrated
/// N=300), not the generous ~2x gap the raw ratio implied -- treat this bench
/// as informational, not a hard CI gate. It is `#[ignore]`d because it is an
/// opt-in wall-clock benchmark, never part of a normal `cargo test` run (see
/// the file header) -- a separate reason from that margin, not the same one.
///
/// THE POST-FIX NUMBER IS ITSELF NOISY -- read it as a smoke test, not a
/// measurement. Five runs of this
/// exact bench, same machine, same code, back to back: 0.883, 0.538, then
/// four more targeted at the spread: 0.865, 0.832, 0.740, 0.827. The
/// baseline (`stats`) ratio held rock stable across those last four --
/// 1.452 to 1.464, a 0.8% spread -- so essentially ALL of the variance is
/// in the numerator, the 300-row `list --limit 1` reading itself: 237ms,
/// 238ms, 267ms, 241ms.
///
/// The 0.538 outlier is diagnosable, not just noisy: it implies a RAW ratio
/// of 0.783 -- the 600-row case finishing FASTER than the 300-row case.
/// That is not physical for a monotonic-in-N workload; it means the 300-row
/// numerator reading that run was itself an outlier on the slow side, and
/// the ratio's SCORING is inverted from what "improvement" should mean here
/// -- run 3 in the four-run set (0.740, the "best" score) scored best
/// because its small-N case was the SLOWEST of the four, not because
/// anything got faster. A corrected ratio that improves when the small-N
/// reading gets noisier is measuring the noise, not the fix.
///
/// Mechanism: this ratio divides two timings where the denominator
/// (`stats`, ~9 O(N) scans, comparatively heavy and stable) barely moves
/// but the numerator (`list --limit 1`, now O(1) after the LIMIT pushdown
/// and therefore FAST and cheap to perturb) is a small number close to process-spawn and
/// scheduling noise -- a small absolute jitter there is a large relative
/// swing in the ratio. This file's own BASELINE CORRECTION section still
/// describes clearing a "measured between-environment noise floor by 2-3x"
/// -- true of the ~1.2 GAP between defect-state and fixed-state ratios, not
/// of the fixed-state number's own run-to-run stability once that gap is
/// closed and the numerator is tiny.
///
/// Net: keep this as a TRIPWIRE -- it reliably distinguishes "pushdown
/// fires" (well under 1.05, every run observed) from "back to O(table)"
/// (the defect-state ~1.17-1.38 band this file measured before the fix). Do
/// NOT read its number as a measurement of HOW MUCH faster the fix is, and
/// do not quote a specific value (0.54, 0.88, or any other) as evidence of
/// improvement -- that number is not going in the PR body as a figure. Any
/// claim about the SIZE of the win needs replicates across separate runs,
/// not more trials within one run (`trials = 9` above already medians
/// within a run and still produced the whole 0.740-0.883 spread, plus the
/// 0.538 outlier, across different runs).
///
/// A ratio is the right shape here: it is dimensionless, so it does not encode
/// this machine's speed. The bound is deliberately loose -- this catches "we
/// went back to O(table)", not a 15% drift.
#[test]
#[ignore = "benchmark: opt in with --ignored. Asserts a CORRECTED ratio \
            (list_ratio / baseline_ratio, where baseline is `mx memory \
            stats` -- no per-row hydration, but not hydration-free either); \
            since --limit is pushed into the SQL query (budgeted per \
            category) whenever no client-side-only filter is active, this \
            PASSES reliably at this file's calibrated N=300 (well under the \
            1.05 bound on every run observed), eliminating the table-size \
            scaling this bench catches for the bounded case it exercises. \
            Before that fix it held ~1.17-1.38 from the (separate, still-real \
            for the unbounded case) 2N+1 hydration defect. The post-fix \
            number itself is noisy run-to-run (observed 0.54-0.88) -- see \
            the doc comment above for why and don't cite a specific value \
            as a performance figure."]
fn bounded_read_cost_does_not_scale_with_table_size() {
    let dir = TempDir::new().unwrap();
    let n = bench_rows();
    let trials = 9;

    seed(&dir, &seed_jsonl(0, n), "first");
    assert_row_count(&dir, n);
    assert_list_limit_one_returns_one_row(&dir);
    let at_n = median_ms(&dir, trials, &["memory", "list", "--limit", "1"], timed_run);
    let baseline_at_n = median_ms(&dir, trials, &["memory", "stats"], timed_run);

    seed(&dir, &seed_jsonl(n, n), "second");
    assert_row_count(&dir, 2 * n);
    assert_list_limit_one_returns_one_row(&dir);
    let at_2n = median_ms(&dir, trials, &["memory", "list", "--limit", "1"], timed_run);
    let baseline_at_2n = median_ms(&dir, trials, &["memory", "stats"], timed_run);

    // A zero-millisecond measurement means the bench measured nothing, not
    // that the operation was free. Left unguarded, a zero numerator collapses
    // `raw_ratio` (and therefore `corrected_ratio`) to 0.0, which trivially
    // clears the `< 1.05` bound below -- a bench that measured nothing would
    // report success. Fail loudly instead of computing a ratio from it.
    for (label, ms) in [
        ("at_n", at_n),
        ("baseline_at_n", baseline_at_n),
        ("at_2n", at_2n),
        ("baseline_at_2n", baseline_at_2n),
    ] {
        assert!(
            ms > 0,
            "{label} measured 0ms -- this bench measured nothing, not that the \
             operation is instant; investigate before trusting any ratio \
             computed from this run"
        );
    }

    let raw_ratio = at_2n as f64 / at_n as f64;
    let baseline_ratio = baseline_at_2n as f64 / baseline_at_n as f64;
    let corrected_ratio = raw_ratio / baseline_ratio;

    // Reported unconditionally: a benchmark that only speaks when it fails is a
    // benchmark nobody can read the trend out of. Raw AND corrected numbers,
    // pass or fail, every run.
    println!(
        "list --limit 1: {n} rows -> {at_n} ms; {} rows -> {at_2n} ms; raw ratio {:.3}\n\
         baseline (`memory stats` -- no per-row hydration, but ~9 O(N) scans of \
         its own): {n} rows -> {baseline_at_n} ms; {} rows -> {baseline_at_2n} ms; \
         baseline ratio {:.3}\n\
         corrected ratio (raw / baseline): {corrected_ratio:.3}",
        2 * n,
        raw_ratio,
        2 * n,
        baseline_ratio,
    );

    // Gated on the calibrated N. The corrected ratio decays monotonically
    // toward 1.0 as N rises (see CALIBRATED_N and BASELINE CORRECTION above),
    // so asserting this bound at an arbitrary N would go GREEN on a genuinely
    // unfixed `list` simply by raising N far enough -- the exact false green
    // this test exists to prevent, reintroduced through the N knob instead of
    // the metric. Only the calibrated N gets a real assertion; any other N
    // prints its numbers for exploration and is skipped.
    if n != CALIBRATED_N {
        println!(
            "MX_BENCH_ROWS={n} is not the calibrated N ({CALIBRATED_N}); skipping \
             the bound assertion. A pass or fail at this N is not signal -- rerun \
             with MX_BENCH_ROWS unset (or ={CALIBRATED_N}) for the real check."
        );
        return;
    }

    assert!(
        corrected_ratio < 1.05,
        "REGRESSION: cost of `list --limit 1` scaled {corrected_ratio:.3}x \
         (corrected for store-open replay cost) when the table doubled ({n} \
         rows: {at_n} ms list / {baseline_at_n} ms baseline; {} rows: {at_2n} \
         ms list / {baseline_at_2n} ms baseline; raw ratio {raw_ratio:.3}, \
         baseline ratio {baseline_ratio:.3}). `--limit` is pushed into the \
         SQL query (budgeted per category, `list_pushdown_eligible` in \
         `src/helpers.rs`) whenever no client-side-only filter is active --  \
         exactly the case this bench exercises -- which is why this bound \
         holds today (observed 0.54-0.88 across runs, noisy but always well \
         under 1.05 -- see the doc comment above this test for why). A \
         failure here means either the pushdown path stopped firing for this case \
         (check `list_pushdown_eligible` and `fetch_by_categories_budgeted`) \
         or the DB layer regressed back to hydrating the whole table -- file \
         a bug, this is no longer expected.",
        2 * n
    );
}
