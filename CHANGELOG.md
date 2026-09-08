# Changelog

All notable changes to `mx` are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Added
- `mx doors` — trigger-based ambient memory, intended as a Claude Code
  `UserPromptSubmit` hook. Any history/list entry carrying `triggers` becomes a
  *door*: when one of its phrases appears in a prompt, `mx doors hook` prints one
  line with a fragment and a `<key>/kv-<id>` pointer. Subcommands: `hook`,
  `check`, `stats`, `reset`. The hook reads the kv file only — no SurrealDB, no
  network — and **always exits 0**, because on `UserPromptSubmit` an exit of 2
  blocks the turn and erases the typed prompt.
- Entries in `history` and `list` keys gained two optional fields, `triggers` and
  `fragment`, settable with `mx kv push --trigger/--fragment` and
  `mx kv update --trigger/--fragment`. Both are omitted from the serialized entry
  when unset, so existing data files load and round-trip unchanged. On `update`,
  `--trigger` replaces the whole list and `--trigger ""` clears it;
  `--fragment ""` clears the override.
- `mx kv triggers [KEY] [--json]` lists every entry carrying triggers, with
  all-time fire counts, and shows what each trigger actually MATCHES on when
  that differs from the stored text (`ayo-` matches as `ayo`).
- `mx kv push`/`update` now vet authored triggers. A trigger with no letters or
  digits (`🦊`, `!!!`) is **rejected** with exit 4 — it could never fire, and
  would otherwise sit in the audit view at `fires=0` looking merely unused. A
  trigger that collapses to a single one- or two-character token (`c++` and `c#`
  both match as `c`) is **warned** about, naming what it became. Warnings and
  notes go to stderr; stdout stays machine-readable. `--trigger ""` remains the
  clear gesture and is never treated as a dead trigger.

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
- `mx kv` write commands now hold an exclusive advisory lock (`flock`) across
  the whole load-mutate-save cycle, on a sidecar `<data>.lock` file beside the
  data file. Every `mx kv` invocation reads the entire JSON store, mutates it in
  memory, and rewrites the whole file; with no lock, two overlapping writers
  each saved a snapshot taken before the other's change and one write was
  silently lost. Reproduced at 8 concurrent `kv push` calls, where 4 of 9
  expected entries survived. The lock lives on a sidecar rather than on the data
  file because `save` publishes by rename, so a lock held on the pre-rename
  inode guards a file the next writer never opens. Read-only commands (`get`,
  `last`, `since`, `dump`, `search`, `random`, `count`) release the lock
  immediately after loading and take none for the rest of the command: the
  atomic rename already gives them a whole-file snapshot, and holding it would
  stall writers for the length of a read, which under `--memory` includes a
  SurrealDB round trip. The wait for the lock is **bounded at 2 s** and then
  fails with a message naming the lock file and how to find the holder; a
  blocking `flock` would turn one wedged `mx kv` (a suspended shell job, a
  process killed with the descriptor still open) into a silent freeze of every
  `mx kv` call on the machine. A command that fails before it can touch the data
  file no longer creates the data directory or an empty lock file. **No change**
  to any command's stdout, `--json` output, or exit codes on success; concurrent
  writers now queue instead of racing, and a wedged lock exits 1 with a
  diagnostic on stderr instead of hanging.
- `mx doors stats --since` no longer reports a door as "never fired" merely
  because its only fires fall outside the window. Never-fired is computed over
  the whole log; `--since` narrows the counts only. That list is the signal used
  to prune bad doors, so scoping it made it lie.
- `mx doors hook` accepts a `session_id` that arrives as a JSON number rather
  than failing the whole payload and going silent for that prompt.
- `mx kv` id errors now name the expected form: `invalid ID '4UW1oq' -- use a
  numeric index, or the stable id with its prefix: kv-4UW1oq`.

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
- **Removed `mx memory trigger-check` and `mx memory trigger-reset`** (Issue
  #246). Their matching engine survives in `src/triggers.rs` and is what `mx
  doors` runs on; only the graph-backed storage and CLI are gone. The
  session fired-state file (`MX_TRIGGER_FIRED_PATH`, default
  `/tmp/wonka-triggered-fired.json`) is no longer read or written. The
  `triggers` field on knowledge entries is unaffected: `mx memory add/update/show`
  still author and display it.
- `triggers::stem_tokens(raw)` is now `triggers::tokens(raw, stem: bool)`, and
  `match_triggers`/`match_entries` take the same flag. Doors match with stemming
  **off** — the English Snowball stemmer folds Tagalog "ayos" onto "ayo", which
  would open an `ayo-` door on unrelated text.
- Library-internal: `KvStore::push`/`push_with_ts` take an `EntryAttrs` struct and
  `KvStore::update_entry` an `EntryPatch`, instead of positional
  `data`/`memory` arguments. No CLI behaviour changes from this.

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
