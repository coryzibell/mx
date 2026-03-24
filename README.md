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

Key environment variables:

| Variable | Purpose |
|----------|---------|
| `MX_MEMORY_PATH` | Override default SurrealDB storage path |
| `MX_CURRENT_AGENT` | Active agent identity (required for `memory wake`, used as default for `--source-agent`) |
| `MX_SURREAL_MODE` | SurrealDB connection mode (`embedded` or `network`) |
| `MX_CODEX_PATH` | Override default codex archive storage path |
| `MX_STATE_SCHEMA` | Default tensor schema ID (defaults to `crewu`) |
| `MX_USER_NAME` | Display name for user in codex transcripts |
| `MX_ASSISTANT_NAME` | Display name for assistant in codex transcripts |

## Further Documentation

See the [project wiki](https://github.com/coryzibell/mx/wiki) for full documentation on the memory system, encoding details, tensor schemas, and sync workflows.

## Status

Published on [crates.io](https://crates.io/crates/mx). The API surface is evolving.

Licensed under [Apache 2.0](LICENSE).
