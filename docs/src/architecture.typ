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
  [`tract-onnx`], [0.22], [Local vector embeddings via ONNX inference (BGE-Base-EN-v1.5, 768-dim)],
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
   state.rs         # mx state subcommand handler (deprecated)
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
 chunking.rs        # token-aware text chunking for embeddings
 embeddings.rs      # EmbeddingProvider trait, TractProvider
 kv.rs              # KV store engine (schema TOML + data JSON)
 types.rs           # shared domain types (Agent, Category, Project, etc.)
 display.rs         # safe_truncate, formatting helpers
 tensor.rs          # emotional state tensor encode/decode (deprecated, serves mx state)
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
failure modes programmatically. The `KvError` enum covers five typed variants:
`KeyNotFound`, `TypeMismatch`, `SchemaMissing`, `EntryNotFound` (a specific
entry ID was not found within a key), and `AmbiguousId` (an ID prefix
matched multiple entries). Both `EntryNotFound` and `AmbiguousId` map to
exit code 4.


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
  [`model_cache_dir()`], [XDG cache or `$MX_HOME/memory/embed/` when isolated],
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

The same pattern is used by `surreal_root_with`, `model_cache_dir_with`,
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

=== Schema auto-apply

The embedded schema (`schema/surrealdb-schema.surql`) is applied on every
database connection, in both embedded and network mode. All statements use
`DEFINE ... IF NOT EXISTS` and `UPSERT`, so re-application is idempotent and
safe. This means a fresh network-mode database is bootstrapped automatically on
first connection -- no manual schema setup is required.

The `apply_schema` method on `SurrealDatabase` uses the `with_db!` macro so the
same code path runs against both the embedded and network backends.

#note[*Contended init:* when several `mx` processes bootstrap a fresh store
concurrently, embedded SurrealDB can return a transient _"read or write
conflict ... can be retried"_ error during schema application. `apply_schema`
retries these with bounded, jittered backoff (up to 12 attempts, with
per-process-entropy jitter to de-correlate racing writers). Because every schema
statement is `IF NOT EXISTS` / `UPSERT`, retries are idempotent. The happy,
uncontended path is untouched -- retries run only when a conflict is actually
observed.]

==== `MX_SKIP_SCHEMA`

Set `MX_SKIP_SCHEMA=1` (or `true`) to skip schema application at connection
time. This is an escape hatch for environments where the database user lacks
DDL permissions (e.g., a read-only replica or a locked-down network instance
where an admin applies the schema separately). When the variable is set,
a `--verbose` message confirms the skip.

==== `mx migrate`

The `mx migrate` command explicitly applies the schema, ignoring
`MX_SKIP_SCHEMA`. It connects to the database (respecting `MX_SURREAL_MODE`
and all connection variables) and runs the full schema. Use it after upgrading
mx to ensure the remote database has any new tables or indexes, or to
re-apply the schema on an instance where `MX_SKIP_SCHEMA` is normally set.

=== Evolving the schema: the BACKFILL convention <schema-backfill>

There is no separate migration tool and no ordered migration history. The
schema file _is_ the migration: it is replayed in full on every connection and
on every `mx migrate`. Schema evolution therefore happens by editing
`schema/surrealdb-schema.surql` so that re-applying it is always idempotent.

This model has one sharp edge that every contributor adding a field must know
about, because the model is SCHEMAFULL.

==== The SCHEMAFULL-stranding trap

When you add a new *required* field (one whose type is not `option<...>`) to an
existing table, SurrealDB does _not_ retroactively populate it. Every
pre-existing row keeps that field as `NONE`. Reads usually survive, because
projections coalesce the missing value (for example the
`IF chunk_count THEN chunk_count ELSE 0 END` projection in
`knowledge_select_fields()`). The danger is the next *write*: SurrealDB
validates the whole record on any write, so the first time anything touches a
stranded row it throws

```
Found NONE for field X ... but expected a <type>
```

This is how issue \#352 stranded 340 of 368 `knowledge` rows: `chunk_count`
was added as a required `int`, and the pre-chunking rows had no value for it.

==== THE RULE

Whenever you add a required field to an existing table, add a paired,
idempotent backfill `UPDATE` immediately after the `DEFINE`:

```surql
DEFINE FIELD chunk_count ON knowledge TYPE int DEFAULT 0;
-- Backfill: required field added to an existing table strands old rows at NONE.
UPDATE knowledge SET chunk_count = 0 WHERE chunk_count IS NONE;
```

Three requirements make a correct backfill:

+ *Guard with `WHERE <field> IS NONE`.* This makes the statement a no-op once
  applied, so replaying the schema on every connection costs nothing and never
  double-counts.
+ *Backfill, do not dodge.* Do not make the field `option<...>` just to avoid
  the trap. Downstream code assumes the field is present; the backfill is the
  correct fix.
+ *If the backfill computes a value, test the non-empty case.* A constant
  backfill is self-evidently correct, but a computed one is not. Add a
  regression test that asserts the computed value matches the real data, not
  merely that the statement runs. The `chunk_count` backfill computes a count
  by joining `embedding_chunk`, so `test_backfill_chunk_count_*` in
  `src/surreal_db/tests.rs` guards it. Those tests use the `cfg(test)`-only
  `test_exec` / `test_raw_chunk_count` helpers on `SurrealDatabase`, which read
  the raw stored value _without_ the read-coalescing projection so a lingering
  `NONE` cannot masquerade as `0`.

The backfill block at the bottom of `schema/surrealdb-schema.surql` documents
this rule inline and collects every historical backfill as a worked example.

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
- Reinforcement: `reinforce` (increment resonance, update activation metadata),
  `update_activations` (batch-reset decay clocks for search activation)
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
- `embedding` (optional array\<float\>) -- 768-dim vector (BGE-Base-EN-v1.5).
  For chunked entries, this holds a normalized mean vector of all chunk
  embeddings (used by `auto-anchor`).
- `embedding_model` (optional string), `embedded_at` (optional datetime)
- `chunk_count` (int, default 0) -- number of embedding chunks. Zero means the
  entry is unchunked (single embedding). A positive value means the entry was
  split into overlapping chunks stored in the `embedding_chunk` table. This is a
  required field; rows that predate it are repaired by a backfill -- see the
  #link(<schema-backfill>)[BACKFILL convention].

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

Embeddings are 768-dimensional float arrays generated by tract-onnx
(BGE-Base-EN-v1.5, local inference). The search strategy is brute-force cosine
similarity -- no HNSW index. This is deliberate at the current scale; the
schema comment notes to reconsider when the store exceeds 50K vectors or
100ms query latency.

The `EmbeddingProvider` trait in `embeddings.rs` abstracts the embedding
backend. `TractProvider` is the sole implementation. The model cache
location is controlled by `paths::model_cache_dir()`.

==== Two-phase semantic search <two-phase-search>

Semantic search uses a two-phase strategy to cover both unchunked entries and
chunked entries:

+ *Phase 1a*: Query unchunked entries (those with `chunk_count <= 0` or absent)
  by cosine similarity against their `embedding` field. Returns up to `limit`
  results.
+ *Phase 1b*: Query the `embedding_chunk` table by cosine similarity. Returns
  up to `limit * 3` results (over-fetching for deduplication).

Both queries run in a single SurrealDB request (chained statements).

+ *Phase 2 (merge)*: Chunk results are deduplicated by `entry_id`, keeping the
  maximum similarity score per entry. For each unique chunk entry, the full
  `knowledge` record is fetched (with visibility, category, and resonance
  filters applied). The unchunked and chunk results are merged into a single
  scored map: if an entry appears in both result sets, the higher score wins.
  The final list is sorted by score descending and truncated to `limit`.

This design means a long entry surfaces in search results if _any_ 400-token
section is semantically relevant, rather than only when the mean vector (which
averages over all sections) happens to score well.

=== Embedding chunks

The `embedding_chunk` table stores per-chunk embeddings for long entries
(those exceeding 400 tokens). Each row represents one chunk of a chunked
entry:

- `entry_id` (string) -- the `kn-` prefixed ID of the parent knowledge entry
- `chunk_index` (int) -- zero-based position within the entry's chunk sequence
- `chunk_text` (string) -- the decoded text of this chunk
- `token_offset` (int) -- token offset from the start of the original text
- `token_count` (int) -- number of tokens in this chunk
- `embedding` (array\<float\>) -- 768-dim vector for this chunk
- `embedding_model` (string) -- model ID that generated the embedding
- `created_at` (datetime)

The table is indexed on `entry_id` (for bulk deletion) and uniquely indexed on
`(entry_id, chunk_index)` (for upsert). Chunks are deleted and re-created on
every re-embed of the parent entry. When a knowledge entry is deleted, its
chunks are cleaned up on a best-effort basis.

Chunking parameters: 400 tokens per chunk, 100-token overlap (stride 300).
These are defined in `ChunkConfig::default()` in `src/chunking.rs`.

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
description = "Areas of active focus"

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
  [`history`], [Timestamped append-only log with optional `max_entries` cap. Supports `push`, `last`, `since`, `search`, `count`, `random`, `update`, `migrate`. Each entry gets a numeric index and a stable base58 entry ID (`kv-` prefix). Entries can carry optional structured JSON data (`--data` on push/update, `--where` on queries). The `last`, `search`, `count`, and `random` commands accept time-range flags (`--day`, `--month`, `--week`, `--since`, `--from`/`--to`) for date filtering.],
  [`list`], [Ordered list with timestamps. Supports `push`, `pop`, `remove`, `search`, `count`, `random`, `update`, `migrate`. Each entry gets a numeric index and a stable base58 entry ID. Entries can carry optional structured JSON data. The `last`, `search`, `count`, and `random` commands accept the same time-range flags as history.],
  [`state`], [Named fields (like a struct). Supports single-field set (`set <key> <field> <value>`), batch set (`set <key> field=value ...` or `set <key> --json '{...}'`), tensor positional set (`set <key> --json '[...]'`), and `get`. Batch operations validate all fields against the schema before writing.],
)

=== Data (JSON)

The data file at `$MX_HOME/kv/data/{agent}.json` holds current values. All
writes are atomic: serialize to a temp file, fsync, rename. The format is a
flat JSON object keyed by the key names from the schema.

History and list entries are stored as objects with `id` (stable entry ID,
serialized from the `id` field), `hash` (legacy on-disk name for the entry ID,
read via `serde(rename)`), `value`, `ts`, an optional `data` field (arbitrary
JSON object for structured metadata), and an optional `memory` field (a `kn-`
ID linking the entry to a knowledge node in the memory graph). In the Rust
structs, the numeric sequence number is the `index` field (serialized as `id`
on disk) and the stable base58 identifier is the `id` field (serialized as
`hash` on disk). The on-disk names are preserved via `serde(rename)` for
backward compatibility -- no data migration is needed. The entry ID is a short
base58 string generated from `blake3(key + timestamp + index)` via base-d,
providing a stable identifier independent of numeric ordering. The `id`
(entry ID), `data`, and `memory` fields all use `#[serde(default)]` for
backward compatibility -- files written before these fields existed are
back-filled on first load (IDs are generated, data and memory default to
`None`) and saved automatically.

=== Schema mutation

Schema lifecycle operations are exposed at the CLI layer under the
`mx kv schema [list | add | drop | update]` namespace, backed by methods on
`KvStore` (which holds a `schema_path` field alongside the existing
`data_path`). The legacy top-level `kv keys` and `kv rename` verbs survive as
deprecated aliases.

The `add_key_def()` method is the single creation path, shared by both
`kv schema add` and the `push --create <type>` inline shortcut. It validates
the key name (alphanumeric, underscores, hyphens; max 128 chars; no dots),
accepts a fully-formed `KeyDef` so all five `ValueType`s can be declared with
their type-appropriate options, inserts it in-memory, and persists via
`save_schema()` (which round-trips the TOML, dropping comments). Duplicate or
invalid names error. `push --create` constructs a history/list `KeyDef` and
calls the same method -- its user-facing surface stays limited to those two
types, but the engine path is unified (#363 acceptance criterion).

Type-appropriate options are enforced at the CLI layer for both `schema add`
and `schema update`: a `validate_type_options` gate rejects options that do
not apply to the declared/existing `ValueType` (e.g. `--min` on a history key,
`--add-field` on a counter) with exit code 4, before any persist. A no-op
`schema update` (no flags set) is likewise rejected rather than rewriting the
schema file.

A legacy `add_key_to_schema()` method still exists as a standalone engine
helper -- it appends a `[keys.<name>]` block to the TOML without reformatting,
preserving comments, for `history`/`list` only. It is no longer wired to any
CLI verb (it formerly backed `push --create`); it is retained for its existing
tests and as an append-based alternative.

The `drop_key()` method backs `kv schema drop`: it removes both the schema
definition and the stored data entry. It follows the `rename_key` transaction
pattern -- mutate in-memory fully, persist data first then schema, roll back on
data-write failure. The handler enforces `--force` for keys with stored content
(`key_has_content()`) and returns `KeyNotFound` for unregistered keys. Outbound
`kn-` memory links are simply discarded; the memory store is never touched.

The `update_key_meta()` method backs `kv schema update` for *safe* metadata
edits (description, `max_entries`, `min`/`max`, adding an optional data field).
Type changes are not representable here -- they are forbidden and routed to
`kv migrate` at the CLI layer, with no override. A `--name` argument instead
delegates to `rename_key`.

The `rename_key()` method moves a key from one name to another in both the
schema and data files. It validates the new name, checks that the old key
exists and the new key does not, then atomically swaps the in-memory
entries before persisting. Data is written first (higher-value file), then
schema. If the data write fails, in-memory mutations are rolled back. Entry
IDs are stable across renames -- they were hashed from the original key
name at creation time and are never regenerated. It is shared by both
`mx kv schema update <key> --name <new>` and the deprecated
`mx kv rename <old> <new>`.

=== Per-agent keying

The active agent is determined by the `MX_CURRENT_AGENT` environment variable.
Schema and data files are resolved via `paths::kv_schema_path(agent)` and
`paths::kv_data_path(agent)`. The path resolution includes a legacy fallback
to `~/.crewu/kv/` for migration purposes.

=== Memory pointers

KV keys can optionally link to a knowledge entry in the SurrealDB store via a
`kn-` ID reference. This allows an agent to associate fast local state with
richer knowledge graph entries. The `--memory` flag on `get`, `last`, `since`,
`search`, `random`, and `dump` resolves these references and displays the
linked entry.

Memory links exist at two levels: key-level (one pointer per key) and
per-entry (one pointer per history or list entry). Per-entry links are set
via `push --memory` at creation time or `set --id --memory` on existing
entries. When resolving, per-entry memory wins over a legacy `kn-` value
prefix, which wins over the key-level fallback. The `SearchHit` struct
(returned by `last`, `random`, `search`, `since`, and `get --id`) carries the
per-entry `memory` field for the handler to resolve.

`SearchHit` derives `serde::Serialize` to support the `--json` output flag.
The serialized field names are the Rust struct names (`index`, `id`, `value`,
`ts`, `data`, `memory`) -- deliberately different from the on-disk
`serde(rename)` aliases used by `HistoryEntry` and `ListEntry`. The `data`
and `memory` fields use `#[serde(skip_serializing_if = "Option::is_none")]`
so they are omitted from JSON output when not set.


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

#note[This is distinct from the runtime write-boundary `dedup_hash`
(`knowledge::dedup_hash`, W447): `content_hash` is *title-only*, computed at
import/seed time, and never enforced (no write-time check reads it back).
`dedup_hash` is *title+body*, computed on every `mx memory add` /
`add-batch` write, checked against the writer's own same-session entries
BEFORE the write lands, and normalizes case/punctuation/whitespace so
regenerated near-duplicates collapse to the same hash. See "Write-boundary
deduplication" in the #link("memory.html")[Memory] reference. The two hashes
are unrelated and neither replaces the other.]


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
