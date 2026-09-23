# Changelog

All notable changes to `mx` are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Changed — lean embedding projection on render-only reads (#438)

Reads that only render — `memory list`, `memory search` (keyword), `memory
show`, `memory wake`/`wake-fetch`/`recent`, `export md`/`export csv`, the `kv`
memory-pointer resolution, and the internal existence/backup/summary checks
on `add`/`delete`/`edit`/`append`/`prepend`/`restore` — no longer fetch or
deserialize the 768-float `embedding` column. On a 1,443-row list projection
(measured over HTTP) this drops the transferred payload from 26.7 MB to 3.3
MB, most of which was previously spent on a vector the terminal renderer
never prints. The server still reads the whole record either way; the
savings are on the wire and in client-side deserialization, not in server
read cost.

- **`--json` output is unchanged by default** on `list`, `search` and `show`:
  the `embedding` array is still there, byte-identical to before. Tooling
  that reads `embedding == null` from `list --json` to find unembedded rows
  keeps working unmodified.
- **New opt-in flag `--omit-embedding`** on `list` and `search` (and `show`)
  makes `--json` output lean too, emitting `"embedding": null` — the same
  shape a never-embedded entry already has. `embedding_model`,
  `embedded_at` and `chunk_count` are unaffected either way. Ignored under
  `search --semantic`.
- **Semantic search (`--semantic`), `auto_anchor`, `auto-anchor`, `embed`,
  and every read-mutate-write path (`update`/`edit`/`append`/`prepend`/
  `restore`)** are untouched — they still fetch the full vector, exactly as
  before.
- **`upsert_knowledge_async` writes `embedding` when the entry carries a
  vector, or when it is genuinely unembedded (`embedding_model` also
  unset)** — the same discriminator that already separated other optional
  columns, extended with the unembedded case so a fresh entry (or an
  unembedded export line) upserted over an existing row still clears a stale
  vector, exactly as an unconditional write always did. A LEAN READ keeps
  `embedding_model`, so it never trips that clear; that's the guard's actual
  job.
- **`export jsonl` is unaffected** — it stays on the full projection, since
  it is the disaster-recovery path and needs the vector to round-trip into a
  fresh database.

### Fixed — wake ritual failure paths (Wake 464 review)

- **A half-written step no longer wedges a ritual.** The guess row and the
  session advance are written in one transaction; the session write is a
  compare-and-swap on `step`, and rows are keyed `wake_guess:[session_id,
  position]`. A step already logged is answered from the log instead of
  judged again — `replayed: true`, with the LOGGED `guess`, `bucket` and
  `match`, the current token and `next`. The first guess counts. This covers
  a retry after a lost response, the loser of a double submit (previously a raw
  index error), and rows the previous binary wrote without advancing — even
  when the entry that row judged has since been deleted.
- **A lost response can be resumed.** The session records the step its last
  write started from (`prev_step`) and that write's status (`last_status`).
  Retrying with that token returns the same answer with `replayed: true` and
  the token the caller never received, however far the call moved `step`; a
  lost `bloom_missing` / `chunk_truncated` echoes its status (without `bloom`).
  **Token out of sync** now means the token really is stale, and says to retry
  with the token from the last successful response.
- **`match.kind` is monotonic.** `exact` means identical after trimming;
  anything that needed normalizing or fuzzing is `close`. A guess wrapped in
  quotes was previously logged `exact` while the bare guess was `close`. Rows
  logged before this fix can be re-derived from their snapshotted guess and
  phrases.
- **`--wake`** is echoed as `wake` (with `model`) on the begin response and
  every respond payload. It must be positive. `--begin` refuses a wake number
  another session already logged guesses under unless `--force-wake`, and
  warns (`warnings`) when it is below the agent's highest logged wake or more
  than one above it. A
  refused begin no longer bumps activation counts.
- **Completed sessions are kept** with `completed_at` set, not deleted.
- **`--respond` accepts a guess starting with `-`.**
- **`invalid_bloom_id` exits 1** with its JSON on stderr, like every other
  error.

### Changed — BREAKING: `mx memory wake` ritual JSON (#448)

The wake ritual is now **one guess per chunk, made from the title alone, after
which the entry is shown**. There is no hint ladder, no second attempt, and no
status that claims the responder knew anything. Every consumer of the ritual's
JSON must be updated; the shapes below are the whole contract.

- **Statuses.** `--respond` returns `shown` or, when the entry shrank past the
  session's chunk cursor mid-ritual, `chunk_truncated`. The old `remembered`,
  `incorrect` and `revealed` statuses are gone, as are `attempt`, `hint`,
  `match_type` and `derived_phrase_mismatch`.
- **Buckets.** Each judged chunk lands in `unhinted` (the guess string-matched
  a phrase, and the tool had given no hint) or `revealed` (it did not). These
  are the only two values. They name what the tool did, not what the responder
  knew: entries shown earlier in a ritual leak into later guesses, and the tool
  cannot see that — which is why every logged row records its position.
- **Respond payload** gains `bucket`, `guess` and
  `match: {kind: exact|close|none, phrase_index}`, and carries `bloom` with
  `{id, title, phrases, phrase_source, content, chunk?}`. `progress` gains
  `buckets: {unhinted, revealed}`. Two statuses judge no guess and so omit
  `bucket`, `guess` and `match`: `chunk_truncated`, where the entry shrank past
  the session's chunk cursor, and `bloom_missing`, where it was deleted
  mid-ritual — the latter omits `bloom` too, there being nothing left to show.
  Both still advance the step, so the token the caller spent stops verifying,
  and both are counted in `summary.unjudged`. An entry deleted between one call
  and the next is stepped over at the start of the following one, so a caller
  that names the entry it was just handed is answered about that entry rather
  than told it used the wrong id.
- **Final summary** is `{chunks, blooms, buckets: {unhinted: {authored,
  derived, auto}, revealed: {...}}, unjudged}` and nothing else. No total, no ratio, no
  per-entry roll-up string; `summary.blooms_complete` and the `BloomRollup`
  type behind it are both removed. `chunks` counts every step the ritual
  walked, and `unjudged` counts the ones where no guess was judged, so
  `chunks` always equals the bucket totals plus `unjudged` and the difference
  never has to be derived — deriving it is how an "N out of M" score gets
  reinvented.
- **Prompt payload** is `{id, title, phrase_source, chunk?}`. `resonance`,
  `resonance_type` and `wake_phrase_count` are dropped — the last existed only
  to tell a consumer whether `--skip` was legal.
- **`--skip` is removed** and clap rejects it as an unknown flag (#452). It had
  been unreachable since every chunk gained a phrase: it always returned
  `skip_requires_phraseless_bloom`.
- **A session created by an older binary** does not load: such a row has no
  `agent` field, and the loader refuses it and asks for a fresh `--begin`
  rather than walking it and filing every guess under an empty agent. Sessions
  live for one ritual, so at most one is affected per deployment.
- **A `--respond` must come from the agent that began the ritual**, and is
  refused otherwise, writing no row. The session token authorises the session,
  not whoever holds it. A caller naming no agent is refused too: the blooms are
  fetched with the caller's context, and an entry that context cannot see is
  indistinguishable from one that was deleted, so a caller with less visibility
  than the owner would otherwise walk the ritual while silently stepping over
  every entry it could not read.
- **A guess is refused, rather than logged, when it is not usable data.** Over
  2000 characters (counted in characters, not bytes, and never truncated), or
  carrying no alphanumeric content at all — an empty, whitespace-only or
  punctuation-only guess. A refusal writes no row and does not advance the
  session, so the caller can simply guess again.
- **Wake-set flags are rejected on a `--respond` call** instead of being
  silently ignored: `--limit`, `--min-resonance`, `--days`, `--no-activate` and
  `--include-excluded` now conflict with `--respond`, as `--wake` and `--model`
  already did. `--limit` and `--days` became optional arguments (defaults
  unchanged, 20 and 7) so an explicit value can be told from an absent one.
- **An empty wake set caused entirely by the tag exclusion now says so**,
  naming the per-tag counts and `--include-excluded`, instead of reporting a
  bare "No blooms to wake" while the entries sit in the graph.
- Help text: `--respond` is "Submit your one guess for this bloom (2000
  characters maximum)", and `--wake-phrase` is "a cue the title is meant to
  evoke".

### Added
- **`wake_guess` table and a row per guess (#448).** Every `--respond` that
  judges a guess writes exactly one row before the session advances, carrying
  the agent, wake number, model, entry and chunk, both positions in the
  sequence, the title as shown, the guess, the phrase snapshot it was matched
  against, the match kind and index, the bucket, and the SHA-256 of the chunk
  text. **A failed row write fails the respond call** and leaves the session
  where it was: the guess is the data the ritual exists to collect, so it is
  not best-effort. A response that judges no guess writes no row. One row per
  session step is enforced by a unique index on (`session_id`, `position`), so
  a client that retries a step after a failed session update cannot log the
  same guess twice. The similarity columns are defined but left null — a row
  with a null `scored_at` is pending for the scoring pass that lands with
  #449. The table is not reachable from `mx memory export`, which reads the
  knowledge table only.
- **`mx memory wake --wake N` and `--model ID`** (with `--begin`) record the
  wake number and the answering model on every guess row. Both are optional
  because agents other than the one that counts wakes also run the ritual, and
  mx has no way to discover either; rows written without them stay reachable by
  entry and by date. mx does not read a wake counter of its own.
- **Default tag exclusion, and `--include-excluded` to turn it off (#448).**
  Entries tagged `archive` or `wake-exclude` are kept out of every layer of the
  wake cascade — core, recent, bridges and the `--min-resonance` path — and
  `--begin` reports per-tag counts in `excluded`, a key that is always present
  and empty when nothing was dropped. **The core layer never lets an exclusion
  cost a kept entry its slot**: it widens its query until it holds a full set
  of entries that survive the exclusion. The recent and bridge layers fetch
  double their quota, which absorbs the ordinary case but is not a guarantee —
  enough excluded entries in one layer can still leave the wake set short.
  Batch-archiving is how you would meet that, since a freshly tagged entry
  counts as recent for seven days. Every layer counts only as far as its quota
  is filled, so an entry ranked below the wake set is never counted. The count
  is an **upper bound** on the entries the exclusion kept out, not an exact
  figure — an excluded entry displaces everything after it, so one reached only
  *because* an earlier exclusion pushed the window down is counted too, though
  it would not have made the cut untagged. Tightening it means comparing
  against the untagged ordering, which for the core layer is already in hand.
  The two tags are separate because a live entry merely kept out of the wake
  set is not an archived copy.
  The match is **exact**: a tag that starts with `archive`, such as
  `archive/2026`, is not excluded. Applying the policy in mx rather than in
  caller-side text keeps it from being dropped in a rewrite.
- `wake_session` rows record `agent`, `wake`, `model_id`, `unhinted_count` and
  `revealed_count`. The superseded counter fields stay defined in the schema so
  existing rows keep validating, and are no longer read or written.

### Fixed
- **Match tolerance is measured in characters on both sides.** `fuzzy_match`
  counted edit distance in characters and divided by a byte length, so for
  3-byte text the denominator was three times too large and the 0.8 tolerance
  widened until a guess with half its characters wrong came back as a close
  match. The same edit ratio now gets the same verdict whatever the text is
  written in. Pre-existing; it starts to matter here because every authored
  phrase is compared and the verdict is written to a log meant to be read back.
- **Nothing matches nothing.** `fuzzy_match` strips every non-alphanumeric
  character before comparing, so a blank guess and a punctuation-only phrase
  both collapsed to the empty string and compared *exactly equal* — logged as
  an exact match in the `unhinted` bucket. An empty normalization on either
  side is now no match.
- **A deleted entry no longer bricks an open ritual.** Every `--respond`
  re-fetches all the session's entries and used to fail on the first one
  missing, so deleting an entry the ritual had already walked past left a
  session that could be neither finished nor cleared. Entries that are gone are
  stepped over; if it is the entry on the table, the response says
  `bloom_missing` and the ritual continues.
- **A `chunk_truncated` response no longer hands back the token it just
  consumed.** The truncation path advanced the entry cursor without ticking the
  step, so the spent token still verified and `summary.chunks` undercounted by
  one per truncation.
- **The final response no longer reports a position past the end.** `progress`
  read "chunk 3 of 2" on the last response of every ritual, in the payload the
  model reads, next to a summary saying otherwise.
- **The core cascade query is bounded again.** It had dropped its SQL `LIMIT`
  and truncated in Rust; since the projection carries `embedding`, that dragged
  a 768-float vector for every high-resonance entry in the graph on every wake.
- **A `wake_order` of zero is no longer read as unset (#456).** Every read path
  selected the field with a truthiness test, and SurrealQL treats `0` as falsy,
  so a stored order of `0` came back as null. The cascade queries derive
  `has_wake_order` from that value, so the entry an author had put *first*
  sorted behind every other ordered one. The field is now read with a presence
  test. The other numeric fields in the same projection fall back to their own
  zero and were never affected. (Unchanged and known: the `?? 999999` sentinel
  that sorts unset orders last would collide with a stored `wake_order` of
  999999.)
- **`mx memory wake --respond` no longer re-runs the cascade (#451).** The
  cascade query and `increment_activation_count` ran before the command
  branched, so every guess incremented the activation count of all ~20 cascade
  entries — a 95-call ritual added about 95 to each. Only `--begin` and the
  plain listing touch activation counts now. Existing inflated values are not
  repaired by this change.
- **Every authored phrase is now matched, not just the first (#450).** Phrases
  were indexed by chunk — chunk *i* was compared against `wake_phrases[i]` only
  — so a single-chunk entry with three phrases could only ever match phrase 0.
  A guess is now compared against every authored phrase, `exact` beating
  `close`, and `match.phrase_index` records which one matched. Authored phrases
  also use the tolerant comparison (case, quotes, whitespace, trailing
  punctuation) that derived phrases already used; the strict rule existed to
  make a game harder, and the match is a recorded fact now, not a gate.
- **The `--min-resonance` wake query has a stable order (#448).** It sorted by
  `resonance DESC` alone, leaving entries of equal resonance in whatever order
  the database returned. It now shares the core query's ordering
  (`has_wake_order DESC, effective_wake_order ASC, resonance DESC`), and all
  four cascade queries gained a final `id ASC` tiebreak. Two rituals over
  unchanged data now produce the same sequence, and `wake_order` decides which
  entry opens it. The core layer's limit is applied after tag exclusion, so a
  dropped entry no longer consumes a slot.
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
