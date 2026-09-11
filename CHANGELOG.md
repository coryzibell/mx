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
