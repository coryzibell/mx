#import "lib.typ": *

#page-header("Architecture", "System internals for contributors.")

This page describes how mx is built. It covers the module structure, dispatch
model, storage backends, and encoding pipeline. The audience is contributors
reading the source code, not users running commands.

== Table of contents

- #link(<overview>)[Overview]
- #link(<module-structure>)[Module structure]
- #link(<command-dispatch>)[Command dispatch]
- #link(<path-management>)[Path management]
- #link(<surrealdb-integration>)[SurrealDB integration]
- #link(<knowledge-graph>)[Knowledge graph data model]
- #link(<codex-archive>)[Codex archive format]
- #link(<kv-store>)[KV store format]
- #link(<base-d-integration>)[Base-d integration]
- #link(<testing-patterns>)[Testing patterns]


// =========================================================================
// OVERVIEW
// =========================================================================

== Overview <overview>

mx is a single-binary Rust CLI built on three pillars:

+ *clap derive* for the command tree -- every subcommand, flag, and validation
  rule is expressed as Rust types in `src/cli.rs`.
+ *SurrealDB* for the knowledge graph -- an embedded SurrealKV database (or
  optional network WebSocket connection) stores entries, relationships, tags,
  embeddings, and metadata.
+ *base-d* for commit encoding -- a separate crate that hashes, compresses, and
  encodes commit messages through randomly selected dictionaries.

The binary is `mx`. There is no library crate; `main.rs` declares modules and
calls into handlers. The Rust edition is 2024.

Key dependencies:

#table(
  columns: (auto, auto, 1fr),
  table.header([*Crate*], [*Version*], [*Role*]),
  [`clap`], [4], [CLI parsing with derive macros],
  [`surrealdb`], [2], [Embedded + WebSocket knowledge store],
  [`base-d`], [3], [Dictionary-based hash/compress encoding],
  [`tokio`], [1], [Async runtime for SurrealDB (multi-thread)],
  [`fastembed`], [5.6], [Local vector embeddings (BGE-Base-EN-v1.5, 768-dim)],
  [`serde` / `serde_json` / `toml` / `serde_yaml`], [1 / 1 / 0.8 / 0.9], [Serialization across JSON, TOML, YAML],
  [`chrono`], [0.4], [Timestamps with serde support],
  [`anyhow` / `thiserror`], [1 / 2], [Error handling (anyhow for handlers, thiserror for typed errors)],
  [`reqwest`], [0.12], [HTTP client for GitHub API calls],
  [`jsonwebtoken`], [10], [JWT signing for GitHub App auth],
  [`pulldown-cmark`], [0.13], [Fence-aware heading extraction],
  [`colored`], [2], [Terminal colors],
)


// =========================================================================
// MODULE STRUCTURE
// =========================================================================

== Module structure <module-structure>

All source lives under `src/`. The top-level modules declared in `main.rs` are:

```
src/
 main.rs            # entry point, Cli::parse(), match on Commands
 cli.rs             # the full command tree (clap derive enums)
 paths.rs           # single source of path truth
 handlers/          # command handler routing
   mod.rs           # top-level dispatchers (pr, github, codex, log, show, etc.)
   memory.rs        # mx memory subcommand handler
   kv.rs            # mx kv subcommand handler
   metadata.rs      # metadata subcommand handler (categories, tags, etc.)
   state.rs         # mx state subcommand handler
 commit.rs          # encoding pipeline (hash + compress + encode)
 knowledge.rs       # KnowledgeEntry struct (the core data model)
 store.rs           # KnowledgeStore trait (abstract storage interface)
 surreal_db/        # SurrealDB implementation of KnowledgeStore
   mod.rs           # SurrealDatabase struct, with_db! macro, RecordId
   connection.rs    # SurrealMode, SurrealConfig, SurrealConnection enum
   knowledge.rs     # SurrealKnowledgeRecord DTO, query hydration
   queries.rs       # backup operations, query helpers
   lookups.rs       # lookup table CRUD (categories, agents, projects, etc.)
   relationships.rs # graph edge operations (relates_to)
   trait_impl.rs    # KnowledgeStore impl for SurrealDatabase
   tests.rs         # integration tests
 codex/             # session conversation archival
   mod.rs           # manifest types, re-exports
   archive/         # the archive pipeline
     mod.rs         # ArchiveRequest, ArchiveOptions, entry points
     include.rs     # IncludeSet (--include flag parser)
     write.rs       # per-session writer, --all driver loop
     sources.rs     # source walkers (subagent discovery, etc.)
     paths.rs       # archive-folder naming, short-ID extraction
     backfill.rs    # vault backfill (--backfill flag)
   export/          # mx codex export pipeline
   index/           # codex indexing
   images.rs        # base64 image extraction from JSONL
   transcript.rs    # conversation.md rendering
   read.rs          # list, read, search operations
   migrate.rs       # v1->v2 archive migration
   notices.rs       # vault-present warnings
 embeddings.rs      # EmbeddingProvider trait, FastEmbedProvider
 kv.rs              # KV store engine (schema TOML + data JSON)
 types.rs           # shared domain types (Agent, Category, Project, etc.)
 display.rs         # safe_truncate, formatting helpers
 tensor.rs          # emotional state tensor encode/decode
 github.rs          # GitHub API operations (cleanup, comments)
 sync/              # GitHub sync (issues, wiki)
 convert.rs         # md2yaml / yaml2md conversion
 session.rs         # deprecated session export (forwards to codex)
 index.rs           # legacy index operations
 helpers.rs         # shared utilities
 wake_chunk.rs      # wake ritual chunking
 wake_ritual.rs     # wake ritual flow
 wake_token.rs      # HMAC-signed wake session tokens
 engage.rs          # interactive wake engage mode
 content_ops.rs     # content editing operations (find/replace, append, etc.)
```

=== Module boundaries

The codebase follows a layered pattern:

+ *CLI layer* (`cli.rs`) -- pure data. No logic, no imports beyond clap.
  Every command variant, flag, and validation constraint is a type.
+ *Handler layer* (`handlers/`) -- orchestration. Reads CLI args, calls into
  domain modules, formats output. Handlers own `println!` and `eprintln!`.
  They do not own business logic.
+ *Domain layer* (`commit.rs`, `knowledge.rs`, `store.rs`, `kv.rs`,
  `codex/`, `embeddings.rs`, `tensor.rs`) -- the actual work. Pure functions
  where possible, side effects isolated to well-defined boundaries (git
  subprocesses, database calls, filesystem writes).
+ *Infrastructure layer* (`surreal_db/`, `paths.rs`, `github.rs`) --
  external integrations. SurrealDB, filesystem, GitHub API.


// =========================================================================
// COMMAND DISPATCH
// =========================================================================

== Command dispatch <command-dispatch>

The dispatch path is:

```
main() -> Cli::parse() -> match cli.command { ... }
```

`main.rs` is small by design. It does three things:

+ Emits a legacy-path deprecation note if `MX_MEMORY_PATH` is set.
+ Parses the CLI with `clap::Parser::parse()`.
+ Pattern-matches on the top-level `Commands` enum and calls the appropriate
  handler.

Some commands dispatch directly to domain functions from `main.rs`:

```rust
Commands::Commit { .. } => commit::upload_commit(..),
Commands::Log { .. } => handle_log(..),
Commands::Show { .. } => handle_show(..),
```

Others dispatch through `handlers/mod.rs`:

```rust
Commands::Memory { command } => handle_memory(command, cli.verbose),
Commands::Kv { command } => handle_kv(command, cli.verbose),
Commands::Codex { command } => handle_codex(command),
```

The handler functions in `handlers/mod.rs` then match on the subcommand enum
and call into domain modules. For example, `handle_codex` matches on
`CodexCommands::Archive`, `CodexCommands::Export`, etc., and routes each to the
appropriate function in `codex::archive`, `codex::export`, or `codex::read`.

=== The `Commit` command

The `Commit` variant is handled inline in `main.rs` rather than through a
handler, because it has two distinct modes selected by the `--encode-only` flag:

+ *Normal mode*: calls `commit::upload_commit()` with the message, stage/push
  flags, and display preferences.
+ *Encode-only mode*: calls `commit::encode_commit_message()` with explicit
  title and body text, prints the result, and exits. No git state is touched.

=== Exit codes

Most commands exit 0 on success or propagate an `anyhow::Error` (which prints
the error chain to stderr and exits non-zero). The `kv` subcommand is the
exception: it uses typed exit codes (0 = OK, 1 = key not found, 2 = type
mismatch, 3 = schema missing, 4 = invalid input) so callers can distinguish
failure modes programmatically.


// =========================================================================
// PATH MANAGEMENT
// =========================================================================

== Path management <path-management>

`src/paths.rs` is the single source of truth for every filesystem path mx
touches. The module is deliberately the _only_ file in the codebase that calls
`dirs::home_dir()`. Every other module that needs a path calls a function from
`paths.rs`.

=== The base directory

All paths derive from `mx_home()`, which resolves once per process via
`OnceLock`:

+ If `MX_HOME` is set and non-empty, use it.
+ Otherwise, use `~/.mx/`.

The result is cached for the lifetime of the process.

=== Derived paths

Each subsystem has its own function in `paths.rs`:

#table(
  columns: (auto, auto),
  table.header([*Function*], [*Returns*]),
  [`mx_home()`], [`$MX_HOME` or `~/.mx/`],
  [`kv_schema_path(agent)`], [`$MX_HOME/kv/schema/{agent}.toml`],
  [`kv_data_path(agent)`], [`$MX_HOME/kv/data/{agent}.json`],
  [`surreal_root()`], [`$MX_SURREAL_ROOT` or `$MX_HOME/memory/surreal/`],
  [`codex_dir()`], [`$MX_CODEX_PATH` or `$MX_HOME/codex/`],
  [`fastembed_cache_dir()`], [XDG cache or `$MX_HOME/memory/embed/` when isolated],
  [`memory_seed_agents_dir()`], [`$MX_HOME/memory/seed/agents/`],
  [`memory_seed_knowledge_dir()`], [`$MX_HOME/memory/seed/knowledge/`],
  [`state_schemas_dir()`], [`$MX_HOME/state/schemas/`],
  [`swap_dir()`], [`$MX_HOME/swap/`],
  [`sync_cache_dir(repo)`], [`$MX_HOME/cache/sync/{repo-slug}/`],
)

=== The `_with()` test-seam pattern <with-pattern>

Pure resolution logic is factored into `_with` variants that take
env-var values as explicit parameters instead of reading `std::env`:

```rust
fn codex_dir_with(env_val: Option<&str>, home: &Path) -> PathBuf {
    if let Some(path) = env_val && !path.is_empty() {
        return PathBuf::from(path);
    }
    home.join("codex")
}

pub fn codex_dir() -> PathBuf {
    codex_dir_with(
        std::env::var("MX_CODEX_PATH").ok().as_deref(),
        mx_home(),
    )
}
```

Tests call the `_with` variant directly with controlled inputs. The public
function is a thin wrapper that reads the env var and passes it in. This keeps
tests parallel-safe (no env-var mutation) and the resolution logic unit-testable
in isolation.

The same pattern is used by `surreal_root_with`, `fastembed_cache_dir_with`,
`resolve_mx_home_with`, and `resolve_kv_path_with`.

=== External paths (read-only)

`paths.rs` also provides helpers for locations owned by other tools that mx
reads but never writes:

- `claude_dir()` -- `~/.claude/`
- `claude_projects_dir()` -- `~/.claude/projects/` (override:
  `MX_CLAUDE_PROJECTS_DIR` for tests)
- `claude_subagents_dir(slug, session)` -- subagent JSONL location
- `claude_sessions_dir()` -- per-PID liveness JSONs
- `claude_history_jsonl()` -- slash-command history
- `claude_mcp_logs_dir(slug)` -- MCP server log parent directory
- `wonka_vault_archives_dir()` -- legacy vault snapshots (`~/.wonka/vault/archives/`)

These are centralized in `paths.rs` so the codex archive source walkers have a
single source of truth for Claude's on-disk layout.


// =========================================================================
// SURREALDB INTEGRATION
// =========================================================================

== SurrealDB integration <surrealdb-integration>

The knowledge graph is backed by SurrealDB. The integration supports two
connection modes:

=== Embedded mode (default)

Uses the `SurrealKV` engine -- a local, file-based key-value store compiled
into the mx binary. No external server process is required. The database files
live at `$MX_HOME/memory/surreal/` (override with `MX_SURREAL_ROOT`).

On first connection, the schema file
(`schema/surrealdb-schema.surql`) is applied via `include_str!`. This is
compiled into the binary -- there is no runtime file read. The schema uses
`DEFINE ... IF NOT EXISTS` and `UPSERT` throughout, making it safe to re-apply
on every startup.

=== Network mode

When `MX_SURREAL_MODE=network`, mx connects to an external SurrealDB instance
over WebSocket (`ws://` or `wss://`). The local `surreal_root` path is unused.
Authentication supports three levels (root, namespace, database), configured
via `MX_SURREAL_AUTH_LEVEL`. Password can be provided directly
(`MX_SURREAL_PASS`) or read from a file (`MX_SURREAL_PASS_FILE`, useful for
agenix-managed secrets on NixOS).

=== Connection architecture

The connection is represented as an enum:

```rust
pub enum SurrealConnection {
    Embedded(Surreal<surrealdb::engine::local::Db>),
    Network(Surreal<WsClient>),
}
```

A `with_db!` macro dispatches across both variants:

```rust
macro_rules! with_db {
    ($self:expr, $db:ident, $body:expr) => {
        match &$self.conn {
            SurrealConnection::Embedded($db) => $body,
            SurrealConnection::Network($db) => $body,
        }
    };
}
```

This allows every query function to be written once and work against both
backends. The `SurrealDatabase` struct wraps the connection and exposes
synchronous methods that internally use a `block_on` bridge over a global
`OnceLock<Runtime>` tokio runtime.

=== The `KnowledgeStore` trait

`src/store.rs` defines the `KnowledgeStore` trait -- the abstract interface for
knowledge storage. `SurrealDatabase` implements this trait in
`surreal_db/trait_impl.rs`. The trait surface includes:

- CRUD: `upsert_knowledge`, `get`, `delete`
- Search: `search` (full-text BM25), `semantic_search` (vector cosine
  similarity)
- Listing: `list_by_category`, `count_by_category`, `list_all`, `count`
- Wake cascade: `wake_cascade` (layered identity retrieval)
- Lookups: categories, agents, projects, sessions, relationships, tags
- Reinforcement: `reinforce` (increment resonance, update activation metadata)
- Backups: pre-mutation content snapshots

The trait exists to decouple handler logic from the storage backend. In
practice, `SurrealDatabase` is the only implementation.


// =========================================================================
// KNOWLEDGE GRAPH DATA MODEL
// =========================================================================

== Knowledge graph data model <knowledge-graph>

The schema lives in `schema/surrealdb-schema.surql` and is compiled into the
binary. It defines a SCHEMAFULL relational-graph model.

=== Core entity: `knowledge`

The central table is `knowledge`. Each row represents one knowledge entry with
the following field groups:

*Identity and content:*
- `title` (string), `body` (optional string), `summary` (optional string)
- `content_hash` (string) -- for change detection during seed/import
- `format` -- `markdown`, `json`, or `stele:*` variants

*Classification (record links):*
- `category` (record\<category\>) -- pattern, technique, insight, gotcha,
  reference, decision, bloom, session
- `source_type` (record\<source\_type\>) -- manual, ram, cache, agent\_session
- `entry_type` (record\<entry\_type\>) -- primary, summary, synthesis
- `content_type` (record\<content\_type\>) -- text, code, config, data, binary
- `source_project`, `source_agent`, `session` -- optional record links

*Visibility:*
- `visibility` -- `public` or `private` (ASSERT constraint)
- `owner` -- agent ID for private entries

*Resonance (wake-up cascade):*
- `resonance` (int) -- importance level, 1--10 with overflow for transcendent
- `resonance_type` -- foundational, transformative, relational, operational,
  ephemeral, session
- `last_activated` (datetime), `activation_count` (int)
- `decay_rate` (float, 0.0--1.0) -- some memories fade, some do not
- `anchors` (array\<string\>) -- IDs of related blooms this entry connects to
- `wake_phrases` (array\<string\>) -- verification phrases for the wake ritual
- `wake_order` (optional int) -- custom sequence position

*Embeddings:*
- `embedding` (optional array\<float\>) -- 768-dim vector (BGE-Base-EN-v1.5)
- `embedding_model` (optional string), `embedded_at` (optional datetime)

=== Graph relations

SurrealDB's graph relations replace traditional junction tables:

#table(
  columns: (auto, auto, auto),
  table.header([*Relation table*], [*Direction*], [*Purpose*]),
  [`tagged_with`], [knowledge -> tag], [Freeform labels],
  [`applies_to`], [knowledge -> applicability\_type], [Scope constraints (language, platform, domain)],
  [`relates_to`], [knowledge -> knowledge], [Inter-entry graph edges],
  [`project_tagged_with`], [project -> tag], [Project-level tags],
  [`project_applies_to`], [project -> applicability\_type], [Project scope],
)

The `relates_to` relation carries a `relationship_type` field
(record\<relationship\_type\>) and is uniquely indexed on the triple (from, to,
type). Relationship types are: related, supersedes, extends, implements,
contradicts, example\_of.

=== Lookup tables

Eight lookup tables provide controlled vocabularies: `category`, `project`,
`agent`, `applicability_type`, `source_type`, `entry_type`, `content_type`,
`relationship_type`, `session_type`, `tag`. Default seed data is applied via
`UPSERT` in the schema file. Users can extend them through
`mx memory categories add`, `mx memory agents add`, etc.

=== Full-text search

A `simple` analyzer (blank + class tokenizers, lowercase filter) powers BM25
search indexes on `title`, `body`, and `summary`. Searches via
`mx memory search` query all three indexes.

=== Vector search

Embeddings are 768-dimensional float arrays generated by FastEmbed
(BGE-Base-EN-v1.5, local inference). The search strategy is brute-force cosine
similarity -- no HNSW index. This is deliberate at the current scale; the
schema comment notes to reconsider when the store exceeds 50K vectors or
100ms query latency.

The `EmbeddingProvider` trait in `embeddings.rs` abstracts the embedding
backend. `FastEmbedProvider` is the sole implementation. The model cache
location is controlled by `paths::fastembed_cache_dir()`.

=== Backups

The `memory_backup` table stores pre-mutation content snapshots. Before any
update, edit, append, prepend, or delete operation, the current content is
written to a backup row. Backups reference entries by plain string ID (not a
record link) so they survive entry deletion.


// =========================================================================
// CODEX ARCHIVE FORMAT
// =========================================================================

== Codex archive format <codex-archive>

The codex is the session conversation archive. `mx codex archive` captures
Claude Code sessions from `~/.claude/projects/` into permanent storage at
`$MX_HOME/codex/`.

=== Archive directory layout

Each archive is a directory named with the pattern:

```
{date}_{short-session-id}[_{counter}]
```

For example: `2026-04-30_abc12345` or `2026-04-30_abc12345_2` for incremental
saves.

Inside each archive directory:

```
{archive}/
  manifest.json       # metadata (version, timestamps, counts, checksums)
  session.jsonl        # raw session JSONL (unless --clean)
  conversation.md      # clean markdown transcript (when --clean or migrated)
  images/              # extracted base64 images (v2+)
    image_001.png
    image_002.png
  agents/              # subagent session JSONLs (when --include subagents)
    agent-{uuid}.jsonl
```

=== Manifest

The manifest is a JSON file tracking archive metadata. The current write
version is 5. All fields added since v2 are `Option` so older archives
deserialize cleanly.

Key fields:

- `version` -- manifest format version (2--5)
- `session_id` -- the Claude session UUID
- `archived_at`, `session_start`, `session_end` -- timestamps
- `project_path` -- the working directory of the session
- `message_count`, `agent_count` -- summary statistics
- `agents` -- array of `AgentInfo` (id, file, message count)
- `size_bytes`, `checksum` -- integrity data
- `image_count`, `images` -- v2: extracted image metadata
- `has_clean_transcript` -- v3: whether `conversation.md` exists
- `user_name`, `assistant_name` -- v4: configurable speaker names
- `source_breakdown` -- v5: per-sidecar byte counts

=== The `IncludeSet`

The `--include` flag on `mx codex archive` controls which optional source
artifacts are captured. It parses a comma-separated string into a struct with
boolean fields:

- `subagents` (default: true) -- capture subagent session JSONLs
- `mcp` -- capture MCP server logs
- `tool_output` -- capture `/tmp` tool outputs
- `history` -- capture `history.jsonl` slice
- `all` / `none` -- shortcuts

=== Source walkers

The archive pipeline uses source walkers to discover files for capture.
Currently `sources.rs` implements subagent discovery
(`find_agent_sessions`). The other source types (MCP, tool-output, history)
are declared in the `IncludeSet` but their walkers are pending implementation
in future PRs.


// =========================================================================
// KV STORE FORMAT
// =========================================================================

== KV store format <kv-store>

The KV store (`src/kv.rs`) is a lightweight local state engine for agents.
No networking, no database -- just a TOML schema file and a JSON data file
per agent.

=== Schema (TOML)

Each agent's schema lives at `$MX_HOME/kv/schema/{agent}.toml` and declares
the keys, types, constraints, and defaults:

```toml
[keys.commit_count]
type = "counter"
min = 0

[keys.recent_files]
type = "history"
max_entries = 50

[keys.current_task]
type = "string"
default = ""

[keys.focus_areas]
type = "list"

[keys.session_state]
type = "state"
fields = ["mode", "context", "priority"]
```

Supported types:

#table(
  columns: (auto, 1fr),
  table.header([*Type*], [*Behavior*]),
  [`counter`], [Integer with optional `min`/`max` bounds. Supports `inc`, `dec`, `set`, `get`],
  [`string`], [Simple string value. Supports `set`, `get`],
  [`history`], [Timestamped append-only log with optional `max_entries` cap. Supports `push`, `last`, `since`, `search`, `count`, `random`. Each entry gets a numeric ID and a stable base58 hash ID (`kv-` prefix). Entries can carry optional structured JSON data (`--data` on push, `--where` on queries). The `last`, `search`, `count`, and `random` commands accept time-range flags (`--day`, `--month`, `--week`, `--since`, `--from`/`--to`) for date filtering.],
  [`list`], [Ordered list with timestamps. Supports `push`, `pop`, `remove`, `search`, `count`, `random`. Each entry gets a numeric ID and a stable base58 hash ID. Entries can carry optional structured JSON data. The `last`, `search`, `count`, and `random` commands accept the same time-range flags as history.],
  [`state`], [Named fields (like a struct). Supports `set <key> <field> <value>`, `get`],
)

=== Data (JSON)

The data file at `$MX_HOME/kv/data/{agent}.json` holds current values. All
writes are atomic: serialize to a temp file, fsync, rename. The format is a
flat JSON object keyed by the key names from the schema.

History and list entries are stored as objects with `id`, `hash`, `value`,
`ts`, and an optional `data` field (arbitrary JSON object for structured
metadata). The `hash` field is a short base58 string generated from
`blake3(key + timestamp + id)` via base-d, providing a stable identifier
independent of numeric ordering. Both `hash` and `data` use
`#[serde(default)]` for backward compatibility -- files written before these
fields existed are back-filled on first load (hashes are generated, data
defaults to `None`) and saved automatically.

=== Per-agent keying

The active agent is determined by the `MX_CURRENT_AGENT` environment variable.
Schema and data files are resolved via `paths::kv_schema_path(agent)` and
`paths::kv_data_path(agent)`. The path resolution includes a legacy fallback
to `~/.crewu/kv/` for migration purposes.

=== Memory pointers

KV keys can optionally link to a knowledge entry in the SurrealDB store via a
`kn-` ID reference. This allows an agent to associate fast local state with
richer knowledge graph entries. The `--memory` flag on `get`, `last`, `since`,
`search`, and `dump` resolves these references and displays the linked entry.


// =========================================================================
// BASE-D INTEGRATION
// =========================================================================

== Base-d integration <base-d-integration>

The `base-d` crate (version 3) provides the encoding layer. It is used in
three places:

=== `commit.rs` -- the encoding pipeline

When `mx commit` runs:

+ `get_staged_diff()` captures the output of `git diff --staged`.
+ `encode_hash_with_registry()` hashes the diff bytes with a random hash
  algorithm and encodes the hash through a random dictionary. This produces the
  commit title.
+ `encode_compress_with_registry()` compresses the commit message with a
  random compression algorithm and encodes the compressed bytes through a
  second random dictionary. This produces the commit body.
+ A footer tag is assembled: `[hash_algo:title_dict|compress_algo:body_dict]`.
+ If both dictionaries are the same (dejavu), the marker `whoa.` is appended.
+ All parts are validated for unsafe characters (NUL, C0/C1 controls). If
  validation fails, the entire encode is retried with freshly rolled
  dictionaries, up to 5 attempts.
+ `git_commit()` writes the three-part message (title, body, footer) as the
  commit message.

The `EncodedCommit` struct captures all parts:

```rust
pub struct EncodedCommit {
    pub title: String,
    pub body: String,
    pub footer: String,
    pub dejavu: bool,
    pub title_dict: String,
    pub body_dict: String,
}
```

=== `handlers/mod.rs` -- the decoding pipeline

`mx log` uses a four-phase architecture:

+ *Parse* -- raw CLI arguments (received as trailing varargs) are parsed into a
  structured `LogOptions` with separate fields for count, display mode
  (`Compact`, `Full`, `Oneline`, format presets, or custom format string), diff
  mode (`None`, `Stat`, `ShortStat`, `Patch`), decorate preference, and filter
  arguments. Custom `--format` strings and `--graph` are detected here and
  trigger a passthrough to raw `git log` with a stderr note.
+ *Harvest* -- a single `git log` call with a structured format string
  retrieves commit metadata (full hash, short hash, decorations, parents,
  author, date, committer, commit date, subject, body). Each commit body is
  decoded via `try_decode_commit_body()`.
+ *Attach diffs* -- if a diff mode was requested, a second `git log` call
  retrieves the diff output. Each diff block is matched to its corresponding
  commit by hash and attached as a string field.
+ *Render* -- the display mode selects a renderer. Each renderer prints the
  decoded message with the appropriate header format, followed by any attached
  diff output.

The `-n`/`--count` and `--full` flags are not clap-managed -- they are parsed
internally from the trailing varargs, following the same pattern as `mx show`.

`try_decode_commit_body()` scans for the last footer-shaped line (validated
against the known compression algorithm vocabulary). Everything above the
footer is the encoded payload; everything below is trailing content (dejavu
markers, user-appended notes). `commit::decode_body()` looks up the dictionary
from the footer, decodes, and decompresses. The scan uses a "last wins"
heuristic: if multiple footer-shaped lines appear (e.g., from a user-amended
commit that quotes a prior footer), the last one is used.

`handle_show()` uses a two-pass approach: Pass 1 retrieves commit metadata
and the encoded message (with `--no-patch`), decodes it, and prints the
header. Pass 2 retrieves the diff output (with `--format=""`) and streams it
as-is. Passthrough detection skips decoding entirely for `ref:path` syntax
(file content viewing) and `--format`/`--pretty` (user-controlled output).

=== `commit.rs` -- PR merge encoding

`mx pr merge` follows the same pipeline but sources the diff from
`gh pr diff` and the message from the PR title and body. The encoded message is
passed to `gh pr merge --subject ... --body ...`.

=== `knowledge.rs` -- content hashing

`KnowledgeEntry` uses base-d's hash encoding for content hashing (via
`base_d::hash` and `base_d::encode`), producing the `content_hash` field used
for change detection during seed/import operations.


// =========================================================================
// TESTING PATTERNS
// =========================================================================

== Testing patterns <testing-patterns>

=== The `_with()` seam

The primary testing pattern in the codebase is the `_with()` test seam
described in #link(<with-pattern>)[Path management]. Any function that reads
from the environment or calls `dirs::home_dir()` is split into:

- A `_with(...)` variant that takes all external inputs as parameters (pure
  function).
- A public wrapper that reads the environment and delegates.

Tests call the `_with` variant directly, avoiding all process-global state.
This means the test suite runs safely in parallel without `#[serial]` except
for the handful of tests that must observe the public wrapper's env-var
behavior.

=== `serial_test`

Tests that mutate process environment (e.g., clearing `MX_CLAUDE_PROJECTS_DIR`
to observe the default fallback) are marked with `#[serial]` from the
`serial_test` crate. These are a small minority -- the `_with()` pattern
eliminates the need for serialization in most cases.

=== `proptest`

The `proptest` crate is available in dev-dependencies for property-based
testing. It is used selectively where input domains are large (e.g., Unicode
boundary testing for `safe_truncate`).

=== Round-trip encoder tests

The `try_decode_commit_body_tests` module in `handlers/mod.rs` tests the
encode-decode round trip by calling `encode_commit()` with known inputs and
verifying that `try_decode_commit_body()` recovers the original message. An
`encode_until` helper retries encoding with different random dictionaries until
a predicate is satisfied (e.g., dejavu vs. non-dejavu), filtering out
dictionary/codec pairings that produce unsafe output or fail round-trip.

=== KV store tests

The KV engine uses the same `_with()` approach for path resolution
(`resolve_kv_path_with`). Store tests operate on temp directories and never
touch the user's real `~/.mx/kv/` state.

=== SurrealDB integration tests

The `surreal_db/tests.rs` module contains integration tests that open a
temporary embedded SurrealKV database, apply the schema, and exercise the full
`KnowledgeStore` trait surface. Each test gets an isolated database directory.
