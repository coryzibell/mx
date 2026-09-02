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
- `mx commit` now verifies that the encoded body decodes back to the original
  message before committing, and re-rolls the codec pair when it does not.
  `validate_encoded_output` only ever checked that the output was *safe* (no
  NUL, no control characters), never that it was *readable* — so dictionaries
  whose base-d codec does not round-trip passed validation and committed
  permanently unreadable messages. Measured at ~6% of encodes (29/500), which
  matches the ~20% of unreadable commits observed in `~/.crewu` history. The
  check is deliberately generic rather than a blacklist of known-bad
  dictionaries, so a newly-broken codec is caught without first being
  identified by hand. Costs one decode per commit. Failures report as
  `roundtrip ...` on the existing retry line, alongside the NUL/control
  reasons.
- `mx commit` no longer draws dictionaries whose alphabet contains a
  whitespace character (`base45`, `uuencode`). Such an encoding cannot survive
  a commit message: `git commit -m` runs `--cleanup=whitespace` and strips
  trailing whitespace from every line, and mx trims independently on both the
  encode and decode paths, so a symbol that IS a space is deleted in transit
  and the payload becomes unrecoverable — a write-time loss the round-trip
  check above can detect but never repair. Implemented as a categorical
  property test on the alphabet, not a list of names, so a whitespace-bearing
  dictionary added later is excluded automatically. Rejection happens at the
  draw rather than in the validation loop, so it does not consume one of the
  bounded encode attempts.
- Embedded schema application now retries transient SurrealDB
  "read or write conflict … can be retried" errors with jittered backoff
  (bounded, `IF NOT EXISTS`-idempotent). Fixes flaky failures when several `mx`
  processes initialize a fresh store concurrently (surfaced by the integration
  test suite under parallel load). No change on the happy path — retries only
  run on a contended init.
- Single-id knowledge lookups now resolve their target as a direct
  `type::thing('knowledge', $id)` record reference in the `FROM`/`UPDATE`
  position, instead of a table scan filtered by `WHERE meta::id(id) = $id`
  (record lookup, backing `mx memory show`) or a one-element `WHERE id IN
  $ids` (the activation-count bump that accompanies it). Contracts are
  unchanged: an empty id still resolves to `Ok(None)`/`Ok(())` rather than
  erroring, and an id with no matching row is still a no-op. Part of #415 --
  this is the record-lookup half only. `list`'s separate per-row hydration
  cost (two additional queries per hydrated row, scaling with table size
  regardless of `--limit`) is a known, still-open cost this PR does not fix.

### Changed
- `mx log` and `mx show` no longer silently print the raw encoded blob when a
  commit body fails to decode. Every decode error — unknown dictionary, decode
  failure, failed decompression, bad UTF-8 — previously collapsed into
  `Err(_) => passthrough` with no reason and no marker, which is why the codec
  round-trip bug above went unnoticed for months. The reason is now captured
  and rendered as a `[decode failed: <reason>]` marker on the affected line.
  Passthrough behavior is otherwise unchanged: the raw text is still shown and
  `mx log` remains usable across ranges that contain broken commits. Exit codes
  are unchanged. The marker is surfaced by **all three** renderers —
  `mx log` (oneline/compact), **`mx log --full`**, and `mx show`. `--full` is
  the case that matters most and was the easiest to miss: what it prints for an
  undecodable commit is the one-way *title hash*, not a message, so without the
  marker a permanently unreadable commit renders as an entirely normal one.
  Reasons are de-duplicated and length-capped so they annotate the line rather
  than consume it.
- Documented the `--min-resonance` basis divergence (Issue #404): `wake`
  filters on **raw** stored resonance, while `list`/`search` filter on
  time-decayed **effective** resonance. Behavior is unchanged; the flag help
  text now states which basis each command uses until an explicit
  `--resonance-basis raw|decayed` flag lands in #404.
