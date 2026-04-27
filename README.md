# mx

A Swiss army knife for Claude Code and multi-agent toolkits.

A Rust CLI providing encoded git operations, a SurrealDB-backed knowledge graph, session archival, GitHub sync, and emotional state tensors. Designed for use with Claude Code, but works with any multi-agent workflow that needs persistent memory, encoded commits, or session management.

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

```bash
# Last 10 commits, decoded
mx log

# Last 20 commits with full details
mx log -n 20 --full
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
mx codex save

# Archive with clean markdown transcript only (no raw JSONL)
mx codex save --clean

# Archive all unarchived sessions
mx codex save --all

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

### Environment Doctor

```bash
mx doctor
mx doctor --json
```

## Base-d Encoding

Commits made with `mx commit` look unreadable in raw `git log` output. This is intentional. The commit message is encoded using [base-d](https://crates.io/crates/base-d):

- **Title**: A hash of the diff, encoded through a randomly chosen dictionary. The title is a fingerprint, not human text.
- **Body**: The actual commit message, compressed and then encoded through another random dictionary.
- **Footer**: A bracket-delimited line indicating which algorithms and dictionaries were used, e.g. `[sha256:ocean|zstd:forest]`.

If both title and body happen to draw the same dictionary, the footer includes `whoa.` -- a dejavu marker.

Use `mx log` to read commits. Use `git log` if you enjoy puzzles.

## Configuration

Filesystem layout (everything lives under `$MX_HOME`, default `~/.mx/`):

```
~/.mx/
├── kv/
│   ├── schema/{agent}.toml     # override MX_KV_SCHEMA
│   └── data/{agent}.json       # override MX_KV_DATA
├── state/
│   └── schemas/{id}.yaml       # default ID: "tensor"; CLI --schema flag
├── memory/
│   ├── surreal/                # override MX_SURREAL_ROOT
│   ├── embed/                  # only when MX_ISOLATE_FASTEMBED is set
│   └── seed/
│       ├── agents/             # for `mx memory seed agents`
│       └── knowledge/          # for `mx memory seed knowledge`
├── codex/                      # override MX_CODEX_PATH
├── cache/sync/{owner-repo}/
├── artifacts/
└── swap/
```

Key environment variables:

| Variable | Purpose |
|----------|---------|
| `MX_HOME` | Base directory (default `~/.mx/`) -- the only filesystem root |
| `MX_CURRENT_AGENT` | Active agent identity (required for `memory wake`, used as default for `--source-agent`) |
| `MX_SURREAL_ROOT` | Override SurrealDB storage root (default `$MX_HOME/memory/surreal/`) |
| `MX_SURREAL_MODE` | SurrealDB connection mode (`embedded` or `network`) |
| `MX_CODEX_PATH` | Override codex archive storage path (default `$MX_HOME/codex/`) |
| `MX_KV_SCHEMA` | Override kv schema path (supports `{agent}` placeholder) |
| `MX_KV_DATA` | Override kv data path (supports `{agent}` placeholder) |
| `MX_ISOLATE_FASTEMBED` | When set, store fastembed model cache under `$MX_HOME/memory/embed/` instead of the shared XDG cache |
| `MX_USER_NAME` | Display name for user in codex transcripts |
| `MX_ASSISTANT_NAME` | Display name for assistant in codex transcripts |

Removed in favor of CLI flags / renames:

- `MX_MEMORY_PATH` -- renamed to `MX_SURREAL_ROOT`. Setting the old variable now emits a one-line stderr note (will be dropped after one release cycle).
- `MX_STATE_SCHEMA` -- replaced by the `--schema {id|path}` flag on `mx state` subcommands; the default schema ID is now `tensor` (was `crewu`).

## Further Documentation

See the [project wiki](https://github.com/coryzibell/mx/wiki) for full documentation on the memory system, encoding details, tensor schemas, and sync workflows.

## Status

Published on [crates.io](https://crates.io/crates/mx). The API surface is evolving.

Licensed under [Apache 2.0](LICENSE).
