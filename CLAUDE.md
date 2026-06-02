# mx

Rust CLI for memory graph and key-value store operations. Backs a SurrealDB instance for persistent memory, traits, session state, and embeddings across agent sessions.

## Build

```sh
cargo build --release
```

## Key Commands

| Command | Purpose |
|---------|---------|
| `mx memory add` | Persist a memory entry |
| `mx memory list` | List memory entries |
| `mx memory get` | Retrieve a specific entry |
| `mx kv set` | Write a key-value pair |
| `mx kv get` | Read a key-value pair |

The `--mine` flag scopes queries to the calling agent (`MX_CURRENT_AGENT` env var).

## Source Layout

| File/Dir | Purpose |
|----------|---------|
| `src/main.rs` | Binary entry point, command dispatch |
| `src/cli.rs` | CLI argument definitions (clap) |
| `src/handlers/` | Per-command handler implementations |
| `src/store.rs` | SurrealDB store abstraction |
| `src/surreal_db/` | SurrealDB client and query layer |
| `src/kv.rs` | Key-value operations |
| `src/types.rs` | Shared types |
| `src/embeddings.rs` | Embedding support |
| `src/paths.rs` | Path resolution |

## Architecture Notes

- SurrealDB backend. Connection config comes from environment or config file -- check `src/paths.rs` for resolution order.
- Commands are agent-aware via `MX_CURRENT_AGENT`. Most list/get operations support `--mine` to scope results.
- Embeddings and knowledge graph features (`src/embeddings.rs`, `src/knowledge.rs`) are secondary to the core memory/KV surface.

## Maintenance Rule

**Before working in this codebase, read the source layout above.** The handlers directory is where most command logic lives.

If you change the behavior of existing commands, note it clearly in commit messages and verify that callers relying on `MX_CURRENT_AGENT` scoping still work correctly.
