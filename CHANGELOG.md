# Changelog

All notable changes to `mx` are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Added
- `mx memory list` and `mx memory search` now emit a best-effort **stderr**
  hint when the caller's own private entries match the query but are hidden by
  the public-only default (Issue #400). The hint reads
  `note: N private entr(y|ies) of yours matched but ... hidden; use
  --include-private to see them`. It fires only when `MX_CURRENT_AGENT` is set
  and neither `--include-private` nor `--mine` was given. **No change** to
  stdout, `--json` output, or exit codes — the hint is STDERR only and any
  error computing it is swallowed silently. The hint is **suppressed under
  `search --semantic`**: its count uses the BM25 `@@` text predicate, which does
  not agree with vector similarity, so counting there would both under- and
  over-report relative to what `--include-private --semantic` actually shows.

### Fixed
- Embedded schema application now retries transient SurrealDB
  "read or write conflict … can be retried" errors with jittered backoff
  (bounded, `IF NOT EXISTS`-idempotent). Fixes flaky failures when several `mx`
  processes initialize a fresh store concurrently (surfaced by the integration
  test suite under parallel load). No change on the happy path — retries only
  run on a contended init.
- Durable memory writes no longer exit non-zero when a post-write embed or
  anchor step fails. `add`, `update`, `edit`, `append`, `prepend` and `restore`
  commit the entry, then run `auto_embed`/`auto_anchor` as best-effort side
  effects; those were chained with `?`, so a transient failure propagated to
  `main()` and produced a non-zero exit **after the write had already landed** —
  callers read "failure", retried, and duplicated the entry. The side-effect
  failure is now captured instead of propagated: the process exits 0 and the
  entry stays durable. **A genuine write failure — the write itself never
  landing — still exits non-zero; that path is unchanged.** The failure is not
  silent either. `--json` mode on all six write paths now carries
  `embed_deferred` / `anchor_deferred`: string fields present **only** when that
  step actually failed, absent on success and absent on a deliberate
  `--no-embed` / `--no-auto-anchor` skip, which keep their existing `(skipped)`
  notices. Plain-mode callers get the same signal on stderr, and all thirteen
  post-write warnings now name the entry id — `... (entry durable, id=kn-…):
  <error>` — so a deferred embed can be reconciled later without grepping for
  which row it was. **Limits, stated plainly:** `add-batch` hoists its embed
  pass and defers anchoring to the nightly run, so its per-entry side effects
  are always empty, and it has no `--json` mode at all — stderr is its only
  signal surface. `add`'s `--type` fact-routing path ignores `--json`
  (pre-existing), so it surfaces the warning on stderr and nothing in JSON.

### Changed
- Documented the `--min-resonance` basis divergence (Issue #404): `wake`
  filters on **raw** stored resonance, while `list`/`search` filter on
  time-decayed **effective** resonance. Behavior is unchanged; the flag help
  text now states which basis each command uses until an explicit
  `--resonance-basis raw|decayed` flag lands in #404.
