# Changelog

All notable changes to `mx` are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Changed — `mx doors` people map, `agent:` readers, pre-burn, `--expires` (doors P3, #489)

- **Behaviour change (D3): an unmapped Matrix sender no longer opens house
  doors.** A channel body now opens doors by who sent it. A sender mapped in
  `doors_people` with `"house":true` places `{house, person:<name>}`, one
  mapped with `"house":false` places `{person:<name>}`, and an unmapped
  sender places only `{person:<localpart>}` (the text between `@` and the
  first `:`, ASCII-lowercased), or `∅` (pinned only) when the id has no
  usable localpart: not a valid name (a non-ASCII letter included, so the
  Kelvin sign never folds to `k`), longer than 64 bytes, or **equal to the
  `person` of any mapped entry** (unmapped `@alice:evil.example` never opens
  the `person:alice` doors of a mapped `@alice:example.org`). **With
  `doors_people` empty that is every channel sender**, so
  no house door opens from Matrix until the map is populated. Map a person
  before authoring `person:<name>` doors for them: until `<name>` is mapped,
  any unmapped sender with that localpart, on any server, opens those doors.
  A mode never touches a channel body. Typed text and pastes route as before.
- **Harness blocks no longer pair across a channel or paste boundary.** A
  harness opener and closer (#478) pair only when both sit outside every
  channel or paste body, or both inside the same one, so a
  `<system-reminder>` typed, pasted or sent by one sender can no longer pair
  with a closer in another sender's body or a paste and carry that text into
  a typed (or that sender's) segment. A harness block that wraps whole channel
  or paste blocks still removes them. A tag left unpaired loses only the tag,
  as before. Known regression: a raw, unclosed channel (or paste) opener
  quoted inside a harness body now counts as a block, so that harness block
  leaks its text and the next sender's block can be relabeled as the quoted
  sender. Claude Code scrubs these tags in subagent output (#500).
- **Upgrade before authoring the first `expires`, `person:` or `agent:`
  door.** An older `mx` doing a read-modify-write of kv drops fields it does
  not know, so it would silently strip `expires` from every entry it
  rewrites, and an older `mx` refuses `person:`/`agent:` scope values.
- **Schema lines needed.** The map is a state key whose field names are open
  (Matrix user ids), so it declares no `fields`; the fire log gains an
  optional `reader`:

  ```toml
  [keys.doors_people]
  type = "state"
  description = "Doors people map: Matrix user_id -> {\"person\",\"house\",\"owner\"}."

  [keys.doors_fired.data.reader]
  type = "string"
  required = false
  ```

  Pre-burn rows use the existing `doors_fired.data.origin` field. Each new
  field is written only where the schema declares it, as before.
- **`doors_people`**: field name = full Matrix `user_id`, value = a JSON
  object string `{"person":"<name>","house":true|false,"owner":true}`
  (`house` and `owner` default to false). A sender resolves on its `user_id`
  alone, by exact match; display names never count. An entry that does not
  parse, or whose `person` is not a valid name, leaves that sender unmapped,
  silently. Set one with
  `mx kv set doors_people @alice:example.org '{"person":"alice","house":true}'`.
- **Typed text gains `person:<owner>`** for each mapped entry with
  `"owner":true`, in every mode: `{house, person:<owner>}` in house and ours
  mode, `{book:<slug>, person:<owner>}` in book mode. No owner entry, no
  change.
- **`person:<name>` and `agent:<name>` scope values** are accepted (a name is
  `[a-z0-9][a-z0-9._-]*`; an agent name may also carry `:`). `person:<n>`
  admits through the matching person place. `agent:` values are the reader
  axis: OR within an axis, AND across axes, and a scope of only `agent:`
  values has the default place `house`.
- **The reader axis.** The reader is the hook payload's `agent_type`, else
  `$MX_CURRENT_AGENT`, else `none`, lowercased; the main thread is a payload
  with no `agent_id`. A door with `agent:` values fires only for a reader it
  names, main thread or not. A door without them fires only on the main
  thread, so a subagent's prompt opens only doors that name its agent. This
  holds for pinned doors too. A plain session's main thread (no
  `agent_type`, no `agent_id`) sees exactly what it saw before.
- **`mx doors check --agent A`** reads as `A` on the main thread, as
  `claude --agent A` would. The default reader for `check` is
  `$MX_CURRENT_AGENT`, else `none`.
- **The budget pool follows the place that admitted the hit**: `book:*`
  spends the book pool, `house` and `person:*` the house pool; a pinned door
  spends its segment's pool as before. Existing doors are unaffected; it
  matters for book-mode typed text, which can now carry both.
- **Pre-burn is on by default.** `mx kv push --trigger …` and
  `mx kv update --trigger …` (setting or replacing non-empty triggers) write
  one `doors_fired` row for `$CLAUDE_CODE_SESSION_ID` with
  `"origin":"authored"`, so a door does not fire back into the session that
  wrote it. **`--fire-here`** skips it, for notes-to-self; it needs
  `--trigger`. No pre-burn without a session id, on `--trigger ""`, or when
  the session already has a row for that door. If the schema cannot take
  `origin` the pre-burn is skipped with one stderr line, and a failed
  pre-burn write is one stderr warning; the command still exits 0. A
  subagent shares its parent's session id, so a door it authors is
  pre-burned for the parent too.
- **Pre-burn rows are not fires.** They dedup, but least-fired-first
  ordering, `mx doors stats` totals, `never fired` and `mx kv triggers`
  `fires=` all leave them out. `doors stats` prints
  `N pre-burned at authoring (not fires)`, and `--json` has `pre_burned`.
- **`--expires WHEN` on `mx kv push` and `mx kv update`**: `<n>h`, `<n>d`,
  `<n>w` (from now) or an RFC 3339 time, stored as absolute UTC in a typed
  `expires` field. On push it needs `--trigger`. `--expires ""` clears it on
  update, and `--expires` on its own is a complete update. Any other form
  (`-1d` included) is a usage error (exit 2) naming
  the accepted forms; a time already past is stored with an
  `already expired` warning. A door is expired once now reaches its
  `expires`, and a stored value that does not parse counts as expired.
  Expired doors are silent in the hook and in `doors check`.
- **`mx kv triggers`** shows `person:`/`agent:` scope values like any other,
  tags an entry `[exp:<date>]` or `[expired]`, and `--json` carries
  `expires` and `expired`. **`mx doors stats`** lists expired doors (key/id,
  expiry, first trigger) under `expired:`, and `--json` has `expired`.
- **`mx doors check --explain`** prints a `reader <name> (main thread|subagent)`
  line after the mode line, shows each channel segment as
  `channel(<user_id> mapped|unmapped)` with its places, and lists a door the
  reader axis or its expiry filtered out as HELD ending
  `held by reader` / `held by expired`. `--json` adds `reader`,
  `main_thread`, a segment's `mapped`, and a door's `held_by`.
- **A shared-trigger warning at write.** After `kv push`/`kv update` with
  `--trigger`, or `--scope` or `--expires` on an entry that has triggers,
  each of the entry's trigger forms carried by at least the per-prompt
  budget (`$MX_DOORS_BUDGET`, else 2), and by at least two, live doors whose
  scopes overlap prints one stderr warning naming the form, the count, the
  budget and the other doors. At the budget it says the form fills it, so any
  other door matching the same prompt defers; over it, that some of these
  will defer. A lone door never warns.
  Scopes overlap when either is pinned, they share a place value (books by
  slug), or one has `house` or `book:*` and the other `person:*`; and they
  miss on the reader axis only when both name agents and no name is shared.
  Warning only; the write always lands.

### Added — `mx doors` match grammar: `p + q`, affix wildcards, `--exclude` (doors P4, #490)

- **Upgrade before authoring the first exclusion.** An older `mx` doing a
  read-modify-write of kv drops fields it does not know, so it would silently
  strip `exclude` from every entry it rewrites.
- **`p + q` in a trigger** fires when every side matches somewhere in the
  same segment, in any order. Each side is a contiguous phrase, as before.
  ` + ` is an operator only as a standalone word, so `c++` and `c++ tips`
  stay plain literals and match as they did (broad, on `c`). Flat N-way AND
  (`a + b + c`) is allowed; there is no nesting, no parentheses and no OR
  (several `--trigger`s are already OR). A side that is empty (`p +`, `+ q`,
  `p + + q`, a bare `+`) exits `4`. A `+` glued to one word
  (`study +blarnia`) is still a phrase; it is stored, with a warning on stderr
  that shows the ` + ` spelling.
- **Affix wildcards**: a word written `prefix*` or `*suffix` matches exactly
  one message token that starts or ends with it (the bare literal included).
  It can sit anywhere in a phrase or a side. The literal is one run of letters
  and digits, at least 5 characters after normalization. `*`, `**foo`,
  `*foo*`, `fo*o`, `*foo-bar` and short literals exit `4`, and the message
  names the rule that was broken.
- **`--exclude PHRASE` on `mx kv push` and `mx kv update`** (repeatable): if
  any exclusion matches in a segment, the door is held for that segment. A
  clean match in another admitted segment still fires. Exclusions take the
  phrase/wildcard grammar; ` + ` in one exits `4`, and the Dead/Broad checks
  apply as they do to triggers. Stored normalized and deduplicated as a typed
  `exclude` field next to `triggers` and `scope`. On `update` it replaces the
  whole list; `--exclude ""` clears it, and `--exclude` on its own is a
  complete update. Exclusions on an entry with no triggers are stored with a
  warning, since they do nothing.
- **Dead and Broad per side.** A side with nothing to match on exits `4`. A
  side that is one short token warns, naming the side, since the other sides
  still narrow it.
- **The most specific trigger** for a grammar trigger counts the tokens of all
  its sides, then characters. Plain triggers measure exactly as before, and the
  fire row's `trigger` is the stored string.
- **`mx kv triggers`** shows a grammar trigger's matched form with its ` + `
  and `*` (`frim_bo + *flimbing` matches as `frim bo + *flimbing`), and an
  `exclude: [...]` line under an entry that has exclusions. `--json` carries
  `"exclude"` when one is set.
- **`mx doors check --explain`** ends the line of a door held by an exclusion
  with `excluded by "<phrase>"`, and `--json` gives that door an
  `excluded_by` field. A scope hold is shown as before.
- **Every plain trigger fires as before.** A trigger with no standalone `+`
  and no `*` goes through the same matcher call as 0.1.231, and plain output
  of `kv triggers` and `doors check` is unchanged.

### Fixed — the wake chunker never cuts a chunk of only rules and blank lines (#487)

- **A rule before a heading no longer becomes its own chunk.** The chunker
  takes the latest break of the best kind in each window, so after splitting
  at a `---` it could split again at the heading right below it and hand the
  ritual a `Part N/M` that was just `---`, with nothing to phrase. A break
  that would leave only blank and rule lines before it is now passed over for
  the next kind of break. The same fix stops a paragraph break followed by a
  long paragraph from producing a chunk that is a single newline.
- **A document that ends in a rule no longer gets a rule-only last chunk.** A
  break inside the trailing run of rules and blank lines is passed over for an
  earlier one, so the rule stays with the prose before it. After an oversized
  code block, a filler (or empty) remainder joins the oversized chunk instead
  of standing alone; that chunk was already flagged oversized.
- **Every other plan is unchanged.** A document that never produced such a
  chunk gets the same boundaries, chunk count and oversized flags, so wake-log
  positions still line up across wakes. A run of rules and blank lines about as
  long as the threshold or longer, or a document that is nothing else, can
  still be split into filler chunks.

### Added — `mx doors` scopes, modes and two budgets (doors P2a/P2b, #435)

- **Upgrade before authoring the first scoped door.** An older `mx` doing a
  read-modify-write of kv drops fields it does not know, so it would silently
  strip `scope` from every entry it rewrites.
- **`--scope` on `mx kv push` and `mx kv update`** (repeatable) says where a
  door may fire. Values: `house`, `pinned`, `book:<slug>` and
  `book:<slug>@<n>` (slug `[a-z0-9][a-z0-9-]*`, n a whole number of 1 or more
  with no leading zero). It is a typed entry field next to `triggers` and
  `fragment`, not part of `data`. On `update` it replaces the whole list;
  `--scope ""` clears it, and `--scope` on its own is a complete update.
  Duplicates are dropped, order kept. Anything else exits `4`, names the bad
  value, and writes nothing; `person:` and `agent:` are refused with a note
  that they arrive in a later phase (P3), because nothing routes them yet and
  such a door would never fire. Not on `kv set`.
- **No scope means `house`.** Nothing is migrated, and an entry without a
  scope serializes byte-for-byte as before.
- **`pinned` fires in every place and every mode**, a quarantined paste
  included. Setting it prints one warning on stderr and exits `0`. Keep it for
  safety and health facts.
- **`mx kv triggers` shows `[scope: …]`** on the line of a scoped entry;
  unscoped lines are unchanged. `--scope S` lists only entries whose scope
  holds S (no scope counts as `house`); `book:<slug>` with no position also
  takes `book:<slug>@<n>` at every n. A bad S exits `4` like `push`, and a
  good one that matches nothing says `no triggered entries match --scope S`.
  `--json` carries `"scope"` when one is set.
- **`mx doors mode`** prints the session's mode (`book:walden@3, since <ts>,
  set by <agent>`, or `house, never set`); `mx doors mode house | ours |
  book:<slug>[@n]` writes a mode row and prints the new mode. The session is
  `--session ID`, else `$CLAUDE_CODE_SESSION_ID`; with neither it exits `1`.
  Bad syntax exits `2`. Setting the current mode again still writes a row,
  which starts a fresh epoch (re-arms hints, resets the stale timer). The
  writer is `$MX_CURRENT_AGENT`, else `cli`.
- **Routing by mode.** Typed text places `{house}` in `house` and `ours` mode,
  and `{book:<slug>}` in `book:<slug>` mode: while reading, the user's own
  words about the book open book doors and hold house doors (pinned still
  fires). This departs from the original design, whose typed row kept
  `{house}` in book mode; book vocabulary in typed riffs was firing house
  doors. Channel bodies place `{house}` in every mode. A paste is routed by
  its verdict: in `house` mode a ForeignProse paste is quarantined (`∅`) and
  everything else is `{house}`; in `book:<slug>` mode an Ours paste goes to
  `{house}` and every other paste (ForeignProse, Unsure, too short to judge)
  goes to `{book:<slug>}`; in `ours` mode every paste is `{house}` and nothing
  is ever quarantined (the classifier still runs, so `--explain` shows its
  verdict). A door opens in a segment when any of its scope values names one
  of the segment's places; a pinned door opens anywhere.
- **The spoiler gate.** `book:walden@12` opens on a Walden paste only in mode
  `book:walden@n` with `12 <= n`. `book:walden` (no position) opens at any
  position. A mode with no position is page one: only position-less book
  doors open. Another slug never opens.
- **Two budget pools**, `house` and `book`, each of `--budget`. A door spends
  the pool of the segment its winning hit came from (a quarantined segment
  spends `house`). Within a pool pinned doors go first, then the existing
  order (fewest fires, oldest, key, id); overflow is deferred per pool. With no
  scopes and `house` mode the fired set, order and lines are what 0.1.228
  printed.
- **`MX_DOORS_BUDGET` (#435).** `mx doors hook` and `mx doors check` take the
  budget from `--budget`, else `MX_DOORS_BUDGET` when it parses as a whole
  number (`0` turns doors off), else 2. An unparseable value is ignored
  silently; the hook never fails on it. One value sets both pools.
- **Hints** (hook only), printed after the door lines as single
  `[doors] ` lines, never counted against a budget, each kind at most once per
  mode epoch: `foreign` (house mode, a paste reads as someone else's prose),
  `ours` (book mode, a paste reads as ours) and `stale` (book mode, and no
  mode or foreign-paste row for 6 hours). A hint is recorded before it
  prints, and one that cannot be recorded does not print. A prompt that opens
  no door still gets its hint. In book mode a prompt with a ForeignProse paste
  also writes one `foreign_paste` row, unless the session's latest mode or
  `foreign_paste` row is under an hour old (the row only feeds the stale
  timer). `--dry-run` prints and writes nothing.
  The hook still loads the store once and saves at most once per prompt.
- **`doors_session` is shared by all sessions and capped at its
  `max_entries`**, so a session's mode row can age out, and then that session
  reads as house.
- **Without `doors_session` in the schema nothing changes**: the hook reads
  `house`, writes no session rows, prints no hints and fires doors as before;
  `mx doors mode <x>` exits `1` and prints the schema to add.
- **Fire rows gain `place`** (`house`, `book:<slug>` or `pinned`) when the
  schema declares it as a string, the same guard as `origin`. Rows without it
  read fine everywhere.
- **`mx doors check --mode M`** evaluates under M instead of the session's
  mode and writes nothing to `doors_session`. `--explain` now starts with
  `mode <mode> (--mode | session | default)`, prints each segment's places
  (`{house}`, `{book:walden}`, `∅ (quarantined)`), each door's place and pool
  (`-` for a held door), and `hint: <kind>` for any hint the hook would print
  (never recorded).
  `--json` carries `mode`, `mode_source`, `places`, `place`, `pool` and
  `hints`. `check` without `--explain` never hints.
- **Schema to add** for modes, hints and `place` on fire rows:

  ```toml
  [keys.doors_session]
  type = "history"
  max_entries = 1000
  description = "Doors session state: mode changes, hints shown, and foreign pastes read in book mode."

  [keys.doors_session.data.session]
  type = "string"
  required = true

  [keys.doors_session.data.event]
  type = "string"
  required = true

  [keys.doors_session.data.mode]
  type = "string"
  required = false

  [keys.doors_session.data.by]
  type = "string"
  required = false

  [keys.doors_session.data.kind]
  type = "string"
  required = false

  [keys.doors_fired.data.place]
  type = "string"
  required = false
  ```

### Changed — `mx doors` drops harness notification preambles

- **A `[SYSTEM NOTIFICATION …]` header line and the prose under it no longer
  open doors.** The header is a whole line: trimmed, it starts with
  `[SYSTEM NOTIFICATION` followed by a space, `-` or `]`, ends with `]`, and
  holds no `<`. The line and the lines after it go, up to the first blank line
  or the first line holding a `<`, so a paste or block closer is never eaten.
  A mention mid-sentence is left alone, and a preamble never reaches past the
  next blank line. Harness blocks are then
  stripped as before.

### Added — `mx kv update --memory` (#480)

- **`mx kv update <key> --id <id> --memory kn-...` sets an entry's memory
  pointer.** It lands in the same write as the value, `--data`, `--trigger`
  and `--fragment`, so either all of them change or none do. An entry that
  already has a pointer gets the new one; `--memory ""` clears it. `--memory`
  on its own is a complete update.
- **Pointers are shape-checked** on `mx kv update --memory`,
  `mx kv set <key> --id <id> --memory` and `mx kv push <key> --memory`. A
  non-empty pointer must be `kn-` followed by one or more ASCII letters or
  digits (`kn-1a2b3c4d`). Anything else (`foo`, `kn-`, `kn-x/y`) exits `4`,
  names the bad value, and writes nothing, even when the same command also
  carries a valid new value. The check is shape only: it does not look the ID
  up in the graph, so kv still works without SurrealDB.
- **`set --id --memory` and `push --memory` used to store any string.**
  Key-level `mx kv set <key> --memory` (no `--id`) is unchanged.
- **`push --memory ""` now stores no pointer** instead of an empty one.

### Changed — `mx kv dec` only ever lowers a counter (#476, #457)

- **A decrement that would not lower the counter is an error.** This is a
  behavior change: `dec` on a counter sitting at its `min` used to clamp
  silently and exit `0`. It now exits `5`, prints the key, the current value
  and the value it would have become (and the `min` when that is why) to
  stderr, and writes nothing. The same goes for a stored value below a
  since-raised `min` and for saturation at `i64::MIN`. A fall that the `min`
  clamps (5 `--by 10` under min 0 -> 0) still succeeds.
- **`--by` must be positive.** `dec --by 0` and negative amounts exit `4`
  with a pointer to `mx kv inc`; nothing is written. `--by -3` (with a space)
  now reaches that check instead of failing in the argument parser with exit
  `2`.
- This is the `dec` half of #457; the `inc` half shipped in #453. There is no
  `--expect` on `dec`. `set` is still the way to force a counter to a value.
- A refused `dec` no longer materializes the key's default in memory.

### Changed — `mx doors` quarantines pasted prose that isn't ours (doors P1)

- **The prompt is matched in segments.** After harness blocks are removed,
  the prompt is cut into typed text, terminal pastes (Claude Code's
  `<pasted_content id="X">…</pasted_content id="X">` envelope; only a closer
  with the same id closes it, an opener nothing closes is text) and
  `<channel>` bodies (keyed by `user_id`, never the display name). Scanning
  left to right, whichever opener comes first defines the block and its body
  is opaque to the other kind: a pasted transcript stays one paste, and paste
  tags inside a channel body are plain text. Channel attributes are stripped
  everywhere and never become match text. Phrases no longer match across a
  segment boundary.
- **A pasted block of someone else's prose holds the doors in it.** Only
  collapsed (tagged) pastes are judged. Each one of at least 600 characters is
  classified ForeignProse / Ours / Unsure from its shape (dialogue share,
  paragraph and sentence length, citations, headings, epigraphs against
  lists, code, chat-log headers, chat register, house vocabulary and names).
  A ForeignProse paste is quarantined: no door matches in it. Ours, Unsure
  and pastes under 600 characters behave exactly as before. Known false
  positive: a long first-person essay that never names us reads as foreign.
- **Typed text and Matrix bodies are never classified.** However long, they
  fire exactly as before (in `book:<slug>` mode typed text routes to the
  book; see Routing by mode). A short paste that arrives untagged counts as
  typed text.
- **A door matching in several segments fires once**, with the most specific
  trigger across them (ties go to the earlier segment). Held doors are never
  fired, never recorded and never spend the budget.
- **`origin` on fire rows.** `doors_fired` rows and `check --json` doors
  gain `origin`: `typed`, `paste` or `channel`. It is written only when the
  store's schema declares `doors_fired.data.origin` as a string; a schema
  without it keeps recording rows exactly as before. Legacy rows without
  `origin` still dedup and count in `stats`.
- **`mx doors check` flags:** `--paste FILE` (repeatable) appends the file
  (less one trailing newline) in the paste envelope, id = first 4 hex of
  sha256(`--session`); `--from USER_ID` sends the message as a channel
  block; `--explain` prints each segment's origin, length, verdict, F/O
  scores, signals and places (typed and channel segments say
  `not classified`; in JSON their `verdict` is null with a `reason`), then
  every matched door as FIRED, DEFERRED or HELD with its segment. It implies
  `--dry-run`, and `--json` gives the same structure. The hook never
  computes held doors.
- **`MX_DOORS_NAMES`** (comma-separated) lists the words that mark text as
  ours, matched whole-word and case-insensitive. Unset, only a standalone
  `Q` counts.

### Fixed — `mx doors` ignores harness-injected blocks (#478)

- **Harness blocks no longer open doors.** `<task-notification>`,
  `<system-reminder>` and every `<local-command-*>` block (`stdout`, `stderr`,
  `caveat`, ...) are removed whole, body included, before triggers are
  matched. A trigger word in a subagent report, a reminder or a CLI echo used
  to open a door, spend its per-session dedup slot and eat the prompt's
  budget. Self-closing tags and stray closers go too, and each removed piece
  leaves one space, so the words on either side stay separate.
- **Channel bodies still match.** Harness blocks are removed first, then
  `<channel>` tags are stripped down to their body exactly as before, so a
  Matrix message still opens doors and its sender attributes still don't.
- **Tag mentions mostly stay text.** Only bare tags count (`<name>`,
  `<name/>`, `</name>`; the harness never writes attributes), so prose like
  `the <system-reminder thing -> please fix` is untouched. A backtick-quoted
  tag is literal text too: an opener or self-closing tag right after a
  backtick, or a closer right before one, takes no part in pairing, so a
  report saying `` `<task-notification>` `` is removed whole. A closer pairs
  with the latest unclosed opener of its name; an opener nothing pairs with
  and a stray closer each lose only the tag. Pairing runs in linear time, so
  hundreds of KB of unclosed openers cost no more than balanced input.
- Known edges: (1) a block whose body quotes a tag WITHOUT backticks can pair
  wrongly, so part of that block's text is still matched, and nested blocks
  of the same name leak the outer block's text; (2) when the next
  unbackticked tag after an unbackticked opener with the same name is a
  closer, the text between them is removed whoever wrote them (unless the
  opener already sits inside an earlier removed block), including when that
  closer is quoted inside a different block.

### Changed — `mx kv inc` only ever raises a counter (#453)

- **An increment that would not raise the counter is an error.** `inc` used
  to clamp silently, so a counter at its `max`, above a since-lowered `max`,
  or at `i64::MAX` reported success while staying put. It now exits `5`, prints
  the key, the current value and the value it would have become (and the
  `max` when that is why) to stderr, and writes nothing. A rise that the `max`
  clamps (51 `--by 100` under max 100 -> 100) still succeeds.
- **`--by` must be positive.** `inc --by 0` and negative amounts exit `4`
  with a pointer to `mx kv dec`; nothing is written.
- **`mx kv inc <key> --expect N`.** Refuses the increment (exit `5`, nothing
  written) unless the counter is currently `N`, so a caller that knows the
  true count catches drift when it happens. A never-written key is compared
  against its schema `default`.
- **New exit code `5`:** the command was well-formed but the store's current
  state refused the write.
- **Concurrent `inc` is pinned by a test:** 8 processes on one counter make 8
  increments and print exactly 1..=8. The kv write lock (#429) already made
  this hold; without it, the same test loses increments on every run.
- `set` is unchanged: it is the explicit way to force a counter, and keeps
  clamping silently. (`dec` got the mirror treatment in #476, above.)
- The strict check lives in `KvStore::inc_expecting` (`inc` delegates to it),
  and a refused `inc` no longer materializes the key's default in memory.

### Fixed — chunk phrases no longer leak on chunks 2..M (#469)

- **Authored phrases apply to chunk 1 only.** Every chunk below the
  authored-phrase count used to be matched against the whole authored list,
  which chunk 1's reveal had just shown. Chunk 1 is still matched against all
  of them, and a single-chunk entry behaves exactly as before. Each later chunk
  gets at most one phrase, from its own text: `derived` on an entry with
  authored phrases, `auto` on one without.
- **A later chunk's phrase is body text, not a heading.** Derived phrases used
  to take the chunk's first markdown heading, and chunks usually start at one.
  The phrase is now the first sentence of a paragraph or list item; headings
  (ATX and setext), rules, code, HTML, bold-only lines such as `**Status:**`
  or `**Status**:`, and sentences without a letter are skipped. A chunk with
  no body text at all still falls back to its heading.
- **A later chunk's phrase never repeats an earlier reveal.** A phrase that
  matches anything shown for an earlier chunk of the same entry, as cut to
  length or in full, is replaced by the chunk's next body sentence that
  doesn't.
- A later chunk is prompted with no phrase (`phrase_source: unphrased`, always
  revealed) when it has no sentence, heading or line with a letter to phrase
  from (a body of only `...` or an emoji is still body text, so it doesn't fall
  back to the heading), or when every candidate repeats an earlier reveal.
- **`wake-log` flags rows logged before the fix.** An `authored` row past the
  first chunk was matched against phrases its entry had just revealed. Read
  commands mark it `leaked` (a `leaked` field in JSON, `leaked` after the
  bucket in text); `report` counts such rows in a new `leaked` column instead of
  `authored`, and leaves them out of Axis A `sim_phrase`. Nothing stored
  changes and `score` is unchanged.

### Added — `mx memory wake-log`: scoring and terminal-only reads (#449)

- **`wake-log score`** scores every pending guess row of the calling agent,
  including rows logged before this release: the guess embedding, Axis A
  (`sim_phrase`, `sim_content`, `sim_title`) and Axis B (`sim_prior`,
  `sim_prior_null`, `prior_n`, plus the same three split by substrate into
  `*_same` / `*_cross`). It loads the embedding model once, and not at all when
  nothing is pending. Its stdout is only `{"status","rows","skipped"}`. A row
  that fails stays pending and is retried; a model-load failure exits non-zero
  with nothing on stdout and every row pending. Authored-phrase vectors are
  cached in a new `wake_phrase_embedding` table (keyed by a SHA-256 of model
  and normalized phrase; no phrase text stored), so a repeat ritual embeds only
  each row's guess and title.
- **`wake-log report [--wake N]`, `wake-log bloom ID [--limit N]` and
  `wake-log wake N`** read the scored log, each opening with the Goodhart text
  (also in `report --help`).
- **The terminal-only rule.** The three read commands print only when stdout is
  a terminal. Otherwise they refuse before reading anything, naming the rule,
  unless `--out FILE` is given; then they write the file (JSON for `.json`,
  text otherwise) and print only its path. An `--out` path that exists must be
  a regular file and not the process's own stdout or stderr (`/dev/stdout` is
  refused, whether stdout is a pipe or a file), and the text layout escapes
  control and bidi-control characters in stored strings. This stops accidental and casual
  leakage of scores and guesses into a model's context; it does not stop a
  process that fakes a terminal or reads the `--out` file.
- **Schema:** six additive optional fields on `wake_guess`
  (`sim_prior_same`, `sim_prior_null_same`, `prior_n_same`, `sim_prior_cross`,
  `sim_prior_null_cross`, `prior_n_cross`). No backfill. `--begin`, `--respond`
  and their replays project and print no scoring field.

### Fixed — a guess row records the title that was shown (#467)

- **`title_shown` is the prompted title.** The session records each prompt as
  it is handed out (at `--begin` and with every `next`): its title, including
  any `(Part N/M)` suffix, its `phrase_source` and its chunk total. The guess
  row's `title_shown`, `phrase_source` and `chunk_total` come from that record,
  not from the entry as it is when the guess arrives, so renaming or
  re-chunking an entry mid-ritual no longer files a guess under a title nobody
  saw. The guess is still matched against the entry's current phrases, and the
  ritual still navigates by its current chunks.
- **The session summary counts the prompted phrase source**, the same one the
  row records and a replay of it counts.
- **A resumed or replayed call hands back the recorded `next`**, so a retry
  after a lost response sees the title its guess will be logged under.
- **A guess about an entry no prompt was issued for is refused.** When the
  entry the caller was handed has been deleted, naming a later one returns
  `invalid_bloom_id` with the deleted entry as `expected_id`; responding about
  it (`bloom_missing`) hands out the next prompt. Previously the later entry
  was judged and logged under a title the caller was never shown.
- **Only the prompted entry is answered as deleted.** Naming any other entry
  while the prompted one is gone is refused with the prompted entry as
  `expected_id`, even when that entry was deleted too or nothing is left after
  it; previously such a call skipped the prompted entry, or was refused with no
  `expected_id` to recover from.
- **A response's `bloom` reveal takes `title` and `phrase_source` from the
  prompt**, as the row does, so the original response, the row and any replay
  of it agree. Its `phrases` and `content` are still the ones the guess was
  judged against.
- **A session awaiting a response with no prompt record is refused** with the
  usual "run `mx memory wake --begin`" error. Sessions opened by an earlier
  binary have none; nothing is backfilled. Completed sessions are unaffected.

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
