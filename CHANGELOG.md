# Changelog

All notable changes to `mx` are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Added
- `mx memory add` and `mx memory add-batch` now run a write-boundary
  duplicate check (W447) before every new-entry write. Dedup identity is the
  4-tuple **(session_id, owner, category, normalized title+body hash)** —
  `category` is part of the key (fixing PR #402 finding 1 where an
  identical title+body filed under a different category was wrongly treated
  as the same fact); `tags` are deliberately excluded, so identical content
  re-filed with different tags still dedups as the same fact re-tagged. When
  a candidate in the same `(session, owner, category)` group already
  matches, the write is skipped: the command **exits 0 without writing**.
  On the standard `mx memory add` write path, plain-mode output prints
  `Already saved as <id> (identical entry this session —
  nothing to do).` and `--json` mode returns `{"id": <existing-id>,
  "skipped": true, "duplicate_of": <existing-id>, "status":
  "already_persisted", ...}` instead of the normal write payload;
  `add-batch` has no `--json` flag, and `add`'s `--type` fact-routing path
  ignores `--json`; both print `Already saved: <id> (<title>) — identical
  this session, no action` instead (batch prefixes each line with its
  1-based line index). This is a change to `add`/`add-batch`'s existing
  success semantics, not a new error path. A new `--allow-duplicate` flag
  on `mx memory add` (and an `allow_duplicate` JSONL field on
  `add-batch`) bypasses the gate entirely for an intentional re-add.
  **Limits, stated plainly:** this is an
  in-process, best-effort check, not a database guarantee — read-then-write
  within a single invocation, no DB-level UNIQUE constraint behind it, so two
  concurrent `mx` invocations can still both write the same normalized
  content (a TOCTOU race, accepted by design). Dedup is also bypassed
  entirely for session-less writes (no `session_id` given). A failure to
  look up existing candidates **fails open**: it prints a stderr warning and
  the write still lands, rather than aborting an otherwise-good write over a
  transient read error.
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
