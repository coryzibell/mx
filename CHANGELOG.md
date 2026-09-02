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
- `mx memory search --semantic` and `mx memory list` now return tags and
  applicability on chunked entries, and `--exclude-tags` now filters them.
  Batch hydration filtered with `WHERE in IN $knowledge`, which plans as a
  `union` lookup over the `in` prefix of the composite UNIQUE index on
  `(in, out)` and matches nothing. The bind itself is correct and `in = $one`
  on that same prefix works, so this was not a keyword collision — it is the
  `union` operator over a composite prefix. Both call sites then swallowed the
  empty result with `take(0).unwrap_or_default()` and reported it as "this
  entry has no tags", so chunked entries came back with tags and applicability
  stripped, and `keep_after_exclude(&[], prefixes)` was unconditionally true —
  `--exclude-tags` silently dropped nothing on the chunked path. Unchunked
  entries were unaffected; they filter in SQL and never read hydrated tags.
  Both queries now traverse from the record ids
  (`SELECT meta::id(id) AS entry_id, ->tagged_with->tag.name AS tags FROM
  $knowledge`), which resolves each entry by key and never reaches that index
  lookup, and both `take(0)` calls propagate instead of defaulting. Measured at
  40k edges / 1000 bound ids: 92ms for tags, 132ms for applicability, flat in
  table size. The narrower alternative `WHERE $knowledge CONTAINS in` returns
  correct rows but plans as `Iterate Table` — 18.9–24.4s at that size, growing
  as O(table rows × bound array length).
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
