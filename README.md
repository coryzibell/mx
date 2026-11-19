# mx

A Swiss army knife for Claude Code and multi-agent toolkits.

A Rust CLI providing encoded git operations, a SurrealDB-backed knowledge graph, session archival, GitHub sync, and emotional state tensors. Designed for use with Claude Code, but works with any multi-agent workflow that needs persistent memory, encoded commits, or session management.

**[Full documentation →](https://coryzibell.github.io/mx/)**

## Installation

```bash
cargo install mx
```

Or from source:

```bash
cargo install --path .
```

Requires Rust 2024 edition. The binary is named `mx`.

## Quick Start

### Encoded Git Commits

`mx commit` wraps `git commit` but encodes the message using [base-d](https://crates.io/crates/base-d). The commit title is hashed and the body is compressed, each encoded through a randomly selected dictionary. The result looks like hieroglyphs in `git log` but decodes cleanly with `mx log`.

Like plain `git commit`, `mx commit` does **not** stage changes on its own — it only commits what is already staged, and errors with `No staged changes to commit` if nothing is staged. Pass `-a` / `--all` to stage all changes (tracked and untracked) before committing.

```bash
# Commit only what is already staged (no auto-staging)
mx commit "fix already-staged typo"

# Stage all and commit with an encoded message
mx commit "fix session export crash on empty JSONL" -a

# Stage all, commit, and push
mx commit "add semantic search to memory" -a -p

# See what the encoding produces without committing
mx commit --encode-only --title "refactor store" --body "split surreal and sqlite backends"
```

### Decoded Git Log

`mx log` has full parity with `git log` -- every display flag, format preset, and filter option works with transparent decoding.

```bash
# Last 10 commits, decoded (default)
mx log

# Last 3 commits (-N shorthand, like git log -3)
mx log -3

# One-line format with ref decorations
mx log --oneline

# Full details with diffstat
mx log -n 5 --full --stat

# Format presets: short, medium, full, fuller
mx log --format=fuller -3

# Filter by author, date, path
mx log --author="charlie" --since="1 week ago"
mx log -- src/handlers/

# Full patch output
mx log -p -3
```

### Decoded Git Show

`mx show` is a drop-in replacement for `git show` that transparently decodes encoded commit messages. All `git show` flags work as expected.

```bash
# Show the latest commit, decoded
mx show

# Show a specific commit with diffstat
mx show abc1234 --stat

# Show just the commit message, no diff
mx show --no-patch

# View a file at a specific revision (passes through to git show)
mx show HEAD:src/main.rs
```

### Knowledge / Memory

The memory system is a knowledge graph backed by SurrealDB (or embedded SurrealKV). Entries have categories, tags, resonance levels, embeddings, and relationships.

```bash
# Search knowledge entries
mx memory search "session bootstrap"

# Semantic (vector) search
mx memory search "how to handle state" --semantic

# Add a knowledge entry
mx memory add \
  --category pattern \
  --title "SurrealDB connection retry pattern" \
  --content "When the connection drops, use exponential backoff..." \
  --tags "surrealdb,reliability" \
  --source-agent smith

# Show a specific entry
mx memory show kn-abc123

# List entries filtered by category
mx memory list -c insight

# Statistics
mx memory stats
```

Default categories: `pattern`, `technique`, `insight`, `gotcha`, `reference`, `decision`, `bloom`, `session`. Categories are customizable per-deployment -- run `mx memory categories list` to see available categories.

`mx memory list` and `mx memory search` are **public-only by default** -- your own private entries are omitted unless you pass `--include-private` (or `--mine`). When a public-only query matches private entries you own but hides them, `mx` prints a best-effort hint on **stderr** pointing at `--include-private` (stdout, `--json`, and exit codes are unaffected). See the [memory docs](https://coryzibell.github.io/mx/memory.html) for the full visibility and resonance-basis details.

How resonance shapes what an agent sees at session start, and how the wake sequence logs its recall, is covered in [Resonance and the wake sequence](#resonance-and-the-wake-sequence).

### KV Store

Fast local key-value state per agent. Counters, strings, lists, timestamped history, and structured state fields -- all backed by a TOML schema file and a JSON data file. No networking, no database.

```bash
# Basic operations
mx kv set session_goal "ship the docs"
mx kv inc builds
mx kv push decisions "chose Typst for docs"    # prints: kv-A3fB (1)
mx kv last decisions --count 5

# Batch set multiple state fields at once
mx kv set context goal="done" phase="writing"
mx kv set context --json '{"goal":"done","phase":"writing"}'
mx kv set mytensor --json '[0.4, 0.6, 0.5]'
echo '{"goal":"done"}' | mx kv set context --json -

# Schema management: define, list, edit, and remove keys
mx kv schema add puns --type history --max-entries 500
mx kv schema list                              # replaces 'kv keys'
mx kv schema update puns --description "the good ones"
mx kv schema drop puns --force                 # removes definition + data

# Auto-create a key in the schema and push in one step (inline shortcut)
mx kv push puns "the joke" --create history
mx kv push ideas "wild thought" --create list --max-entries 500

# Structured data on entries
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'

# Query by structured data with --where
mx kv search projects --where status=active
mx kv search projects "DSI" --where status=active
mx kv search projects --where tags=palmtop --where status=active
mx kv last projects --where status=active --count 5
mx kv count projects --where status=active

# Time-range queries on history/list keys
mx kv last shipped --day 2026-04-25
mx kv last shipped --month 2026-04
mx kv last shipped --week 2026-W17
mx kv last shipped --from 2026-04-01 --to 2026-04-15
mx kv last shipped --since 1w
mx kv search shipped "feature" --month 2026-04
mx kv count shipped --day 2026-05-07

# Time range composes with --count (filter first, then limit)
mx kv last shipped --month 2026-04 --count 5

# Entry lookup by numeric index or entry ID
mx kv get shipped --id 35
mx kv get shipped --id kv-A3fB
mx kv get shipped --id 35-64
mx kv get shipped --id 1,kv-A3fB,12

# Remove by entry ID
mx kv remove shipped --id kv-A3fB

# Random sampling from history/list keys
mx kv random shipped --count 5
mx kv random ideas --count 1
mx kv random shipped --count 3 --since 30d

# Update an entry's value or structured data in-place
mx kv update projects "palmtop DSI fix (v2)" --id kv-A3fB
mx kv update projects --id 42 --data '{"status":"done"}'
mx kv update projects --id 42 --data '{"obsolete_field":null}'

# Migrate entries to match current schema data definitions
mx kv migrate projects --dry-run
mx kv migrate projects --prune

# Rename a key (preserves all entries, IDs, and data)
mx kv schema update old_decisions --name archived_decisions
# (the top-level 'mx kv rename' still works but is deprecated)

# Per-entry memory links (bridge KV entries to the knowledge graph)
mx kv push decisions "adopted memory links" --memory kn-abc123
mx kv set decisions --id 17 --memory kn-abc123
mx kv last decisions --count 3 --memory

# JSON output for scripting and jq piping
mx kv last projects --count 5 --json | jq '.[].data.status'
mx kv search projects --where status=active --json | jq 'map(.value)'
mx kv count shipped --json | jq '.count'
```

Every entry gets a stable entry ID (e.g. `kv-A3fB`) alongside its numeric index. Push prints both: `kv-A3fB (42)`. Anywhere a numeric index works, an entry ID also works -- `--id kv-A3fB`, mixed comma lists, remove. Ranges remain numeric only. Old data files are back-filled on first load.

Entries can carry structured JSON data via `--data` on push or update. Query entries by data fields with `--where key=value` (available on `search`, `last`, `random`, `count`). Multiple `--where` flags are ANDed. Supports exact string match, array-contains, and numeric/boolean comparison on top-level fields.

Individual entries can link to the memory graph via `--memory kn-xxx` on `push` (at creation) or `set --id --memory` (on existing entries). Per-entry memory wins over key-level fallback. Pass `--memory` on read commands (`get`, `last`, `since`, `search`, `random`, `dump`) to resolve linked knowledge entries.

Time-range flags (`--day`, `--month`, `--week`, `--since`, `--from`/`--to`) are available on `last`, `search`, `count`, and `random`. All dates are UTC. The `--since` flag accepts relative times (`30d`, `1w`, `2h`, `30m`) and ISO-8601 timestamps. For a standalone relative-time query on history keys, use `mx kv since`.

### PR Merge

```bash
# Squash merge (default) with encoded commit message
mx pr merge 42

# Rebase merge
mx pr merge 42 --rebase

# Standard merge commit
mx pr merge 42 --merge-commit

# Merge now with admin privileges, bypassing branch policy (e.g. REVIEW_REQUIRED)
mx pr merge 42 --admin

# Queue the merge until all branch requirements are met
mx pr merge 42 --auto
```

`--admin` and `--auto` are mutually exclusive and work with any merge mode. `--admin` passes `--admin` through to `gh` for an immediate merge that bypasses base-branch protection. This is the sanctioned path for single-account agent workflows: GitHub forbids approving your own pull request, so an agent committing under one account cannot satisfy a `REVIEW_REQUIRED` policy on its own — `--admin` merges anyway, using admin merge rights. `--auto` queues the merge and lets GitHub complete it once requirements are met; it does not merge immediately, so post-merge cleanup (branch switch and local source-branch deletion) is deferred. In both cases mx still constructs the encoded `--subject`/`--body`, so admin merges produce a decodable commit like any other.

### Session Archival (Codex)

Archives Claude session JSONL files to permanent storage with transcripts, extracted images, and manifests.

```bash
# Archive the current session
mx codex archive

# Archive with clean markdown transcript only (no raw JSONL)
mx codex archive --clean

# Archive all unarchived sessions
mx codex archive --all

# List archived sessions
mx codex list

# Read an archived session
mx codex read <archive-id> --clean

# Search across all archives
mx codex search "memory migration"
```

### GitHub Sync

Pull and push issues/discussions as local YAML files.

```bash
mx sync pull owner/repo
mx sync push owner/repo --dry-run
mx sync labels owner/repo
```

## Resonance and the wake sequence

Two parts of the memory system decide what an agent sees at session start and record how it responds to it. The full reference is in the [memory docs](https://coryzibell.github.io/mx/memory.html).

### Resonance

Every entry carries an integer `resonance` (1-10, higher allowed) and an optional `resonance_type`: `foundational`, `transformative`, `relational`, `operational`, `ephemeral` or `session`.

- **Decay.** `list` and `search` filter on an *effective* resonance that decays weekly from the entry's last activation: 10% per week at resonance 3 or below, 5% at 4-5, 2.5% at 6 and above. `foundational` and `transformative` entries do not decay. `wake` uses the raw stored value.
- **Activation.** `mx memory show` and `search --activate` count as deliberate access: they bump `activation_count` and reset `last_activated`, which restarts the decay clock. Being surfaced by `wake` only bumps the count, so an entry cannot stay fresh just by being shown at every session start.
- **Reinforcement.** `mx memory reinforce` raises the stored resonance (default +1, capped at 10). Adding a relationship reinforces its target the same way, except for `contradicts` and `supersedes`.

`mx memory wake` builds the wake set from three layers, filled in order up to `--limit` (default 20):

1. **Core:** raw resonance 8 or higher, not `ephemeral`.
2. **Recent:** activated within `--days` (default 7).
3. **Bridges:** resonance 5 or higher, anchored to an entry already in core or recent.

Within a layer, entries with an explicit `wake_order` come first, then higher resonance, then ID. The order is deterministic, so the same set is walked in the same sequence every time. Entries tagged `archive` or `wake-exclude` are left out.

### The wake sequence

`mx memory wake --begin` walks the set as recall before reveal. For each entry the agent sees only the title, submits exactly one guess at what the entry says with `--respond`, and is then shown the entry. There is no hint and no second attempt; retrying a step returns the guess already logged.

- **Chunking.** Content over 28,000 bytes (`MX_WAKE_CHUNK_BYTES`) is split at headings, rules or paragraph breaks, never inside a fenced code block. Each chunk is its own step with its own guess, titled `(Part N/M)`.
- **Phrase sources.** Each guess is matched against the step's phrases, and the prompt names where they came from: `authored` (wake phrases written on the entry, first chunk only), `auto` (one phrase extracted from the content when none were written), `derived` (one phrase from a later chunk's own text, chosen so it repeats nothing already revealed) or `unphrased` (a later chunk with nothing usable to match).
- **Buckets, not a score.** A guess that matches a phrase, exactly or after normalization, lands in `unhinted`. Anything else lands in `revealed`. The bucket names what the tool did, not what the agent knew. The final response reports bucket counts split by phrase source, plus chunks walked and steps left unjudged.

### The guess log and scoring

Every judged guess is written as a `wake_guess` row in the same transaction that advances the session: the entry, chunk, position, title as shown, guess, phrases, match and bucket, plus the optional `--wake` label and `--model` identifier given at `--begin`. Rows are scoped to the calling agent and are not part of `mx memory export`.

`mx memory wake-log score` embeds pending rows and stores two axes per guess. **Axis A** (`sim_phrase`, `sim_content`, `sim_title`) compares the guess with the entry. It rises whenever phrases are retuned toward past guesses, so it is a phrase-authoring aid and nothing more. **Axis B** compares the guess with the centroid of the agent's 5 most recent earlier guesses on the same entry and title from other sessions (`sim_prior`), against a baseline of the same centroid built for up to 10 other entries (`sim_prior_null`). The gap between the two is the signal. Axis B is also stored over prior guesses from the same `--model` (`*_same`) and from a different one (`*_cross`), which allows comparison across model versions. In the tool's own words, Axis B "is evidence of stable reaching, not of remembering." The 5 and the 10 are untuned defaults, and Axis B is empty until an entry has history.

### Why the scores are terminal-only

A measure the subject can see stops measuring the subject. A model that can read its score can play to it, and one that can read its earlier guesses will repeat them, which would inflate Axis B directly. So `score` prints only row counts, `--begin` and `--respond` never carry an axis value, and `wake-log report`, `bloom` and `wake` print only when stdout is a terminal. Anywhere else they refuse before reading anything, unless `--out FILE` names a regular file. A model's shell tool is not a terminal; a human's shell is. This guards against accidental leakage. It does not stop a process that deliberately fakes a terminal or reads the `--out` file.

```bash
mx memory wake --begin --wake 3 --model example-model
mx memory wake --bloom-id kn-abc123 --session <token> --respond "one guess from the title"
# repeat --respond with each returned token until the summary
mx memory wake-log score
mx memory wake-log report             # human terminal only
mx memory wake-log bloom kn-abc123    # one entry's guesses across sessions
```

## Configuration

Everything mx writes lives under a single base directory: `$MX_HOME`, default
`~/.mx/`. Each subsystem owns a subdirectory (`kv/`, `state/`, `memory/`,
`codex/`, ...). Move the whole tree by setting `MX_HOME`, or override one
subsystem at a time with vars like `MX_SURREAL_ROOT`, `MX_CODEX_PATH`,
`MX_KV_SCHEMA`, `MX_KV_DATA`, `MX_ISOLATE_MODELS`.

Two env vars were removed in the path-alignment refactor (#259):

- `MX_MEMORY_PATH` -- use `MX_SURREAL_ROOT` instead. Setting the old name now
  emits a one-line stderr note and is otherwise ignored.
- `MX_STATE_SCHEMA` (deprecated) -- replaced by the `mx state ... --schema {id|path}` CLI
  flag. The default schema ID is now `tensor` (was `crewu`). Note: `mx state`
  itself is deprecated; use `mx kv` with structured data instead.

For the full layout, the complete env-var reference (including SurrealDB
connection vars, GitHub App auth, and tuning), legacy-fallback behavior, and
worked examples, see the **[filesystem layout](https://coryzibell.github.io/mx/paths.html)**.

## Documentation

For the complete command reference, configuration guide, and architecture docs, see the [full documentation](https://coryzibell.github.io/mx/).

## Status

Published on [crates.io](https://crates.io/crates/mx). The API surface is evolving.

Licensed under [Apache 2.0](LICENSE).
