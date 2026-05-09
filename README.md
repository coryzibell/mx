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

```bash
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

### KV Store

Fast local key-value state per agent. Counters, strings, lists, timestamped history, and structured state fields -- all backed by a TOML schema file and a JSON data file. No networking, no database.

```bash
# Basic operations
mx kv set session_goal "ship the docs"
mx kv inc builds
mx kv push decisions "chose Typst for docs"
mx kv last decisions --count 5

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

# Entry lookup by ID on history/list keys
mx kv get shipped --id 35
mx kv get shipped --id 35-64
mx kv get shipped --id 1,5,12,35

# Random sampling from history/list keys
mx kv random shipped --count 5
mx kv random ideas --count 1
mx kv random shipped --count 3 --since 30d
```

Entries can carry structured JSON data via `--data` on push. Query entries by data fields with `--where key=value` (available on `search`, `last`, `random`, `count`). Multiple `--where` flags are ANDed. Supports exact string match, array-contains, and numeric/boolean comparison on top-level fields.

Time-range flags (`--day`, `--month`, `--week`, `--since`, `--from`/`--to`) are available on `last`, `search`, `count`, and `random`. All dates are UTC. The `--since` flag accepts relative times (`30d`, `1w`, `2h`, `30m`) and ISO-8601 timestamps. For a standalone relative-time query on history keys, use `mx kv since`.

### PR Merge

```bash
# Squash merge (default) with encoded commit message
mx pr merge 42

# Rebase merge
mx pr merge 42 --rebase

# Standard merge commit
mx pr merge 42 --merge-commit
```

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

## Configuration

Everything mx writes lives under a single base directory: `$MX_HOME`, default
`~/.mx/`. Each subsystem owns a subdirectory (`kv/`, `state/`, `memory/`,
`codex/`, ...). Move the whole tree by setting `MX_HOME`, or override one
subsystem at a time with vars like `MX_SURREAL_ROOT`, `MX_CODEX_PATH`,
`MX_KV_SCHEMA`, `MX_KV_DATA`, `MX_ISOLATE_FASTEMBED`.

Two env vars were removed in the path-alignment refactor (#259):

- `MX_MEMORY_PATH` -- use `MX_SURREAL_ROOT` instead. Setting the old name now
  emits a one-line stderr note and is otherwise ignored.
- `MX_STATE_SCHEMA` -- replaced by the `mx state ... --schema {id|path}` CLI
  flag. The default schema ID is now `tensor` (was `crewu`).

For the full layout, the complete env-var reference (including SurrealDB
connection vars, GitHub App auth, and tuning), legacy-fallback behavior, and
worked examples, see the **[filesystem layout](https://coryzibell.github.io/mx/paths.html)**.

## Documentation

For the complete command reference, configuration guide, and architecture docs, see the [full documentation](https://coryzibell.github.io/mx/).

## Status

Published on [crates.io](https://crates.io/crates/mx). The API surface is evolving.

Licensed under [Apache 2.0](LICENSE).
