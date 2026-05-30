-- =============================================================================
-- Memory Knowledge Base - SurrealDB Schema
-- Version: 1.0
-- SurrealDB-native schema
-- =============================================================================

-- Use namespace and database
-- Note: Executed at connection time in Rust, not in schema file

-- =============================================================================
-- LOOKUP TABLES (Records with metadata)
-- =============================================================================

-- Categories: default seeds below; deployments may customize via 'mx memory categories'
DEFINE TABLE IF NOT EXISTS category SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON category TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON category TYPE datetime DEFAULT time::now();

-- Projects: dotmatrix, mx, base-d, etc.
DEFINE TABLE IF NOT EXISTS project SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON project TYPE string;
DEFINE FIELD IF NOT EXISTS path ON project TYPE option<string>;
DEFINE FIELD IF NOT EXISTS repo_url ON project TYPE option<string>;
DEFINE FIELD IF NOT EXISTS description ON project TYPE option<string>;
DEFINE FIELD IF NOT EXISTS active ON project TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS created_at ON project TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON project TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS project_active ON project FIELDS active;

-- Agents: neo, smith, trinity, etc.
DEFINE TABLE IF NOT EXISTS agent SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON agent TYPE option<string>;
DEFINE FIELD IF NOT EXISTS domain ON agent TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON agent TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON agent TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS agent_domain ON agent FIELDS domain;

-- Applicability types: rust, python, cross-platform, linux-only
DEFINE TABLE IF NOT EXISTS applicability_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON applicability_type TYPE string;
DEFINE FIELD IF NOT EXISTS scope ON applicability_type TYPE option<string>;  -- language, platform, tool, domain
DEFINE FIELD IF NOT EXISTS created_at ON applicability_type TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS applicability_scope ON applicability_type FIELDS scope;

-- Source types: manual, ram, cache, agent_session
DEFINE TABLE IF NOT EXISTS source_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON source_type TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON source_type TYPE datetime DEFAULT time::now();

-- Entry types: primary, summary, synthesis
DEFINE TABLE IF NOT EXISTS entry_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON entry_type TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON entry_type TYPE datetime DEFAULT time::now();

-- Content types: text, code, config, data, binary
DEFINE TABLE IF NOT EXISTS content_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON content_type TYPE string;
DEFINE FIELD IF NOT EXISTS file_extensions ON content_type TYPE option<string>;  -- comma-separated
DEFINE FIELD IF NOT EXISTS created_at ON content_type TYPE datetime DEFAULT time::now();

-- Relationship types: related, supersedes, extends, implements
DEFINE TABLE IF NOT EXISTS relationship_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON relationship_type TYPE string;
DEFINE FIELD IF NOT EXISTS directional ON relationship_type TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS created_at ON relationship_type TYPE datetime DEFAULT time::now();

-- Session types: claude_desktop, agent_task, manual
DEFINE TABLE IF NOT EXISTS session_type SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS description ON session_type TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON session_type TYPE datetime DEFAULT time::now();

-- Tags: freeform labels
DEFINE TABLE IF NOT EXISTS tag SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON tag TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON tag TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS tag_name ON tag FIELDS name UNIQUE;

-- =============================================================================
-- CORE TABLES
-- =============================================================================

-- Sessions
DEFINE TABLE IF NOT EXISTS session SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS session_type ON session TYPE record<session_type>;
DEFINE FIELD IF NOT EXISTS project ON session TYPE option<record<project>>;
DEFINE FIELD IF NOT EXISTS started_at ON session TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS ended_at ON session TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS metadata ON session TYPE option<object>;  -- flexible JSON
DEFINE INDEX IF NOT EXISTS session_type_idx ON session FIELDS session_type;
DEFINE INDEX IF NOT EXISTS session_project_idx ON session FIELDS project;

-- Knowledge entries (main entity)
DEFINE TABLE IF NOT EXISTS knowledge SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title ON knowledge TYPE string;
DEFINE FIELD IF NOT EXISTS body ON knowledge TYPE option<string>;
DEFINE FIELD IF NOT EXISTS summary ON knowledge TYPE option<string>;
DEFINE FIELD IF NOT EXISTS file_path ON knowledge TYPE option<string>;
DEFINE FIELD IF NOT EXISTS content_hash ON knowledge TYPE string;
DEFINE FIELD IF NOT EXISTS ephemeral ON knowledge TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS owner ON knowledge TYPE option<string>;
DEFINE FIELD IF NOT EXISTS visibility ON knowledge TYPE string DEFAULT 'public'
  ASSERT $value IN ['public', 'private'];
DEFINE FIELD IF NOT EXISTS created_at ON knowledge TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON knowledge TYPE option<datetime>;

-- Record links (replaces foreign keys)
DEFINE FIELD IF NOT EXISTS category ON knowledge TYPE record<category>;
DEFINE FIELD IF NOT EXISTS source_project ON knowledge TYPE option<record<project>>;
DEFINE FIELD IF NOT EXISTS source_agent ON knowledge TYPE option<record<agent>>;
DEFINE FIELD IF NOT EXISTS source_type ON knowledge TYPE record<source_type>;
DEFINE FIELD IF NOT EXISTS entry_type ON knowledge TYPE record<entry_type>;
DEFINE FIELD IF NOT EXISTS content_type ON knowledge TYPE record<content_type> DEFAULT content_type:text;
DEFINE FIELD IF NOT EXISTS session ON knowledge TYPE option<record<session>>;

-- Resonance fields for wake-up cascade.
-- The `knowledge` table first shipped without these; they were added AFTER rows
-- already existed (the wake-up cascade / "atomic facts" feature). So on any
-- graph that predates the cascade, every legacy row holds NONE for resonance,
-- activation_count, and decay_rate → the SAME cold-upgrade stranding hazard as
-- triggers/wake_phrases/chunk_count/format (Issue #360). Required + legacy-added
-- ⇒ loosen → backfill → OVERWRITE-tighten. The read path already coalesces these
-- (IF resonance THEN resonance ELSE 0 END, etc. in knowledge_select_fields()),
-- so reads survive — but the next WRITE touching a stranded row aborts
-- whole-record validation with `Found NONE for field 'activation_count' ...
-- but expected a int`, taking the all-or-nothing apply_schema down with it.
-- OVERWRITE (not IF NOT EXISTS): loosen unconditionally so an upgrade from a
-- prior version where the field was strict actually loosens. Backfilled to their
-- DEFAULTs (0 / 0 / 0.0) and tightened in the COLD-UPGRADE BACKFILL block below.
DEFINE FIELD OVERWRITE resonance ON knowledge TYPE option<int>;
-- Note: 'session' was added to resonance_type allowed values as part of the session resonance
-- type feature (PR #148), not the tag query feature. It appears here because both changes
-- landed in the same schema file update.
DEFINE FIELD IF NOT EXISTS resonance_type ON knowledge TYPE option<string>
  ASSERT $value IN [null, 'foundational', 'transformative', 'relational', 'operational', 'ephemeral', 'session'];
DEFINE FIELD IF NOT EXISTS last_activated ON knowledge TYPE option<datetime>;
DEFINE FIELD OVERWRITE activation_count ON knowledge TYPE option<int>;
DEFINE FIELD OVERWRITE decay_rate ON knowledge TYPE option<float>;
DEFINE FIELD IF NOT EXISTS anchors ON knowledge TYPE option<array<string>>;

-- Issue #72: Multiple wake phrases for ritual variety.
-- Required field added after rows existed → cold-upgrade hazard (Issue #360).
-- Defined as option<...> here; backfilled + tightened to strict array<string>
-- in the COLD-UPGRADE BACKFILL block below. See the triggers comment for why.
-- OVERWRITE (not IF NOT EXISTS): loosen unconditionally so an upgrade from a
-- prior version where the field was strict actually loosens (Issue #360).
DEFINE FIELD OVERWRITE wake_phrases ON knowledge TYPE option<array<string>>;

-- Issue #73: Custom wake order (separate from resonance)
DEFINE FIELD IF NOT EXISTS wake_order ON knowledge TYPE option<int>;
DEFINE INDEX IF NOT EXISTS knowledge_wake_order ON knowledge FIELDS wake_order;

-- Issue #246: Trigger keywords for reactive memory injection.
-- Mirrors the wake_phrases field: `array<string> DEFAULT []`.
--
-- COLD-UPGRADE BACKFILL REQUIRED (Issue #360 — corrects an earlier claim here).
-- ---------------------------------------------------------------------------
-- An earlier version of this comment asserted "no backfill is needed" for
-- `triggers`, on the theory that read-path coalescing + write-path binding
-- meant no row was ever stranded. That reasoning is WRONG for a populated
-- pre-existing graph (Issue #360, ~5,283-row graph reported via the hearth
-- federation, Soren/Geoff). Here is the real write path:
--
--   * `DEFINE FIELD ... TYPE array<string>` does NOT validate at define-time.
--     The strict type is only enforced on the NEXT WRITE that touches a row.
--   * On a SCHEMAFULL table, SurrealDB validates the ENTIRE record on ANY
--     write. So when ANY backfill/seed/upsert below first writes a legacy row,
--     the validator inspects every required field — including `triggers`.
--   * Legacy rows that predate this field hold NONE for it (absent == NONE on
--     disk in SurrealDB 2.6.x). The write therefore aborts with
--     "Found NONE for field 'triggers' ... but expected a array<string>", and
--     because apply_schema runs the whole file through one `db.query` with
--     `take_errors()` (all-or-nothing), the entire connect fails.
--   * A bare `UPDATE ... SET triggers = [] WHERE triggers IS NONE` placed AFTER
--     the strict DEFINE does not help: that UPDATE is itself the failing write.
--   * Backfilling BEFORE the DEFINE on a SCHEMAFULL table is silently DROPPED
--     (you cannot `SET` an undefined field on a SCHEMAFULL table).
--
-- THE WORKING SEQUENCE (used here and for every required field added after
-- rows already exist — see "THE RULE" in the BACKFILL section): define the
-- field as `option<...>` first, backfill legacy NONE rows to the default, then
-- tighten to the strict required type with `DEFINE FIELD OVERWRITE`. The
-- tighten MUST use OVERWRITE: `IF NOT EXISTS` silently SKIPS because the field
-- already exists, leaving it loose (`option<...>`) forever.
--
-- ORDERING NOTE: the backfill UPDATE and the OVERWRITE-tighten for this field
-- live together with the other cold-upgrade fields in the COLD-UPGRADE
-- BACKFILL block lower down (just before the SEED DATA), NOT here. The reason
-- is whole-record validation: a backfill UPDATE only succeeds once EVERY
-- required legacy-added field on the row is non-NONE, so all such fields must
-- be defined as `option<...>` first (here, at their natural sites), then
-- backfilled together, then tightened together — no write may happen while any
-- one of them is still NONE.
--
-- This loosen MUST be OVERWRITE, not IF NOT EXISTS: on an upgrade from a prior
-- mx version the field ALREADY EXISTS as the strict `array<string>`, so
-- IF NOT EXISTS would silently skip and the field would stay strict — the
-- backfill below would then die exactly as in #360. OVERWRITE loosens
-- unconditionally (and creates it on a fresh graph).
DEFINE FIELD OVERWRITE triggers ON knowledge TYPE option<array<string>>;
-- NOTE: `knowledge_triggers` INDEX is defined in the COLD-UPGRADE BACKFILL
-- block below, AFTER `triggers` is tightened back to the strict
-- `array<string>` — index definitions must see the field in its final type.

-- DEPRECATED: Kept for backward compatibility during migration
DEFINE FIELD IF NOT EXISTS wake_phrase ON knowledge TYPE option<string>;

-- =============================================================================
-- VECTOR EMBEDDINGS (PR #89)
-- =============================================================================
-- Semantic search via fastembed-rs (BGE-Base-EN-v1.5, 768 dimensions)
-- Strategy: Brute force (no HNSW index) - exact results for Luola's scale
-- Reconsider HNSW when: >50K vectors OR >100ms query latency
--
-- RETRIEVAL PERF DECISION (Issue #361 — decided, do not relitigate)
-- ---------------------------------------------------------------------------
-- 1. Brute-force is deliberate, and exactness is load-bearing.
--    `semantic_search_knowledge` runs the cosine DB-side
--    (`vector::similarity::cosine(...) ORDER BY score LIMIT`), which is exact
--    and O(n) over compact column data with a bounded top-K. We keep it exact
--    because `auto_anchor`'s near-duplicate ceiling depends on exact similarity
--    in the high-similarity region — precisely where approximate ANN/HNSW most
--    often misses. Approximate recall there would cause missed dedups and wrong
--    anchors, so trading exactness for speed is not acceptable at this layer.
--
-- 2. No persistent embedding daemon.
--    The ~1.2-1.3s model load is per-CLI-invocation startup tax, accepted as
--    such for a single-shot CLI. There is no repeated embed within one call, so
--    an in-process cache does not amortize it; only a resident daemon would, and
--    that is a different architectural contract we are not adopting for mx.
--
-- 3. ANN/HNSW deferred behind an explicit breakeven trigger.
--    Revisit a vector index only when scale crosses: >50K vectors OR sustained
--    query latency >100ms (the threshold above). Tradeoffs: ANN gives sub-linear
--    queries but approximate recall, plus write amplification (every mutation
--    updates the index — worse with #346 chunk vectors, multiple vectors/entry),
--    index memory/tuning cost, and awkward deletes. At current scale (~5K
--    vectors) we are ~10x under the trigger.
--
-- 4. The real perf win is #362, not the index.
--    The bulk of measured cost is `auto_anchor`'s application-layer full-graph
--    scan; #362 moves that off the app layer. Land #362 first; only revisit ANN
--    when scale (not a one-time setup cost) crosses the trigger above.
-- ---------------------------------------------------------------------------

DEFINE FIELD IF NOT EXISTS embedding ON knowledge TYPE option<array<float>>;
DEFINE FIELD IF NOT EXISTS embedding_model ON knowledge TYPE option<string>;
DEFINE FIELD IF NOT EXISTS embedded_at ON knowledge TYPE option<datetime>;

-- Embedding chunk count (Issue #346).
-- Required field added after rows existed → cold-upgrade hazard (Issue #360).
-- Defined as option<int> here; backfilled to the per-entry chunk count +
-- tightened to strict `int DEFAULT 0` in the COLD-UPGRADE BACKFILL block below.
-- OVERWRITE (not IF NOT EXISTS): loosen unconditionally so an upgrade from a
-- prior version where the field was strict actually loosens (Issue #360).
DEFINE FIELD OVERWRITE chunk_count ON knowledge TYPE option<int>;

-- Issue #122: Stele encoding format.
-- Required field added after rows existed → cold-upgrade hazard (Issue #360).
-- Defined as option<string> here; backfilled to 'markdown' + tightened to the
-- strict asserted `string DEFAULT 'markdown'` in the COLD-UPGRADE block below.
-- OVERWRITE (not IF NOT EXISTS): loosen unconditionally so an upgrade from a
-- prior version where the field was strict actually loosens (Issue #360).
DEFINE FIELD OVERWRITE format ON knowledge TYPE option<string>;

-- Indexes for common queries
DEFINE INDEX IF NOT EXISTS knowledge_category ON knowledge FIELDS category;
DEFINE INDEX IF NOT EXISTS knowledge_source_project ON knowledge FIELDS source_project;
DEFINE INDEX IF NOT EXISTS knowledge_source_agent ON knowledge FIELDS source_agent;
DEFINE INDEX IF NOT EXISTS knowledge_source_type ON knowledge FIELDS source_type;
DEFINE INDEX IF NOT EXISTS knowledge_entry_type ON knowledge FIELDS entry_type;
DEFINE INDEX IF NOT EXISTS knowledge_content_type ON knowledge FIELDS content_type;
DEFINE INDEX IF NOT EXISTS knowledge_session ON knowledge FIELDS session;
DEFINE INDEX IF NOT EXISTS knowledge_updated ON knowledge FIELDS updated_at;
DEFINE INDEX IF NOT EXISTS knowledge_ephemeral ON knowledge FIELDS ephemeral;
DEFINE INDEX IF NOT EXISTS knowledge_owner ON knowledge FIELDS owner;
DEFINE INDEX IF NOT EXISTS knowledge_visibility ON knowledge FIELDS visibility;

-- Resonance indexes for wake-up queries
DEFINE INDEX IF NOT EXISTS knowledge_resonance ON knowledge FIELDS resonance;
DEFINE INDEX IF NOT EXISTS knowledge_resonance_type ON knowledge FIELDS resonance_type;
DEFINE INDEX IF NOT EXISTS knowledge_last_activated ON knowledge FIELDS last_activated;

-- =============================================================================
-- FULL-TEXT SEARCH
-- =============================================================================

-- Simple analyzer for knowledge search (blank + class tokenizers, lowercase filter)
-- Note: Snowball stemming syntax not supported in SurrealDB 2.4.0, using simpler approach
DEFINE ANALYZER IF NOT EXISTS simple
    TOKENIZERS blank, class
    FILTERS lowercase;

-- Search indexes using simple analyzer
DEFINE INDEX IF NOT EXISTS knowledge_title_fts ON knowledge
    FIELDS title SEARCH ANALYZER simple BM25;
DEFINE INDEX IF NOT EXISTS knowledge_body_fts ON knowledge
    FIELDS body SEARCH ANALYZER simple BM25;
DEFINE INDEX IF NOT EXISTS knowledge_summary_fts ON knowledge
    FIELDS summary SEARCH ANALYZER simple BM25;

-- =============================================================================
-- GRAPH RELATIONS (Replace junction tables)
-- =============================================================================

-- Knowledge <-> Tag relationship
DEFINE TABLE IF NOT EXISTS tagged_with SCHEMAFULL TYPE RELATION
    FROM knowledge TO tag;
DEFINE FIELD IF NOT EXISTS created_at ON tagged_with TYPE datetime DEFAULT time::now();
-- Prevent duplicate edges
DEFINE INDEX IF NOT EXISTS tagged_with_unique ON tagged_with FIELDS in, out UNIQUE;

-- Knowledge <-> Applicability relationship
DEFINE TABLE IF NOT EXISTS applies_to SCHEMAFULL TYPE RELATION
    FROM knowledge TO applicability_type;
DEFINE FIELD IF NOT EXISTS created_at ON applies_to TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS applies_to_unique ON applies_to FIELDS in, out UNIQUE;

-- Project <-> Applicability relationship
DEFINE TABLE IF NOT EXISTS project_applies_to SCHEMAFULL TYPE RELATION
    FROM project TO applicability_type;
DEFINE FIELD IF NOT EXISTS created_at ON project_applies_to TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS project_applies_unique ON project_applies_to FIELDS in, out UNIQUE;

-- Project <-> Tag relationship
DEFINE TABLE IF NOT EXISTS project_tagged_with SCHEMAFULL TYPE RELATION
    FROM project TO tag;
DEFINE FIELD IF NOT EXISTS created_at ON project_tagged_with TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS project_tagged_unique ON project_tagged_with FIELDS in, out UNIQUE;

-- Knowledge <-> Knowledge relationship (the graph!)
DEFINE TABLE IF NOT EXISTS relates_to SCHEMAFULL TYPE RELATION
    FROM knowledge TO knowledge;
DEFINE FIELD IF NOT EXISTS relationship_type ON relates_to TYPE record<relationship_type>;
DEFINE FIELD IF NOT EXISTS created_at ON relates_to TYPE datetime DEFAULT time::now();
-- Unique constraint on the triple (from, to, type)
DEFINE INDEX IF NOT EXISTS relates_to_unique ON relates_to FIELDS in, out, relationship_type UNIQUE;

-- =============================================================================
-- MEMORY BACKUPS (Issue #206 - pre-mutation content snapshots)
-- =============================================================================

DEFINE TABLE IF NOT EXISTS memory_backup SCHEMAFULL;
-- entry_id is a plain string (not record<knowledge>) because backups must
-- survive entry deletion — a deleted entry has no record to reference.
DEFINE FIELD IF NOT EXISTS entry_id     ON memory_backup TYPE string;
DEFINE FIELD IF NOT EXISTS title        ON memory_backup TYPE string;
DEFINE FIELD IF NOT EXISTS body         ON memory_backup TYPE option<string>;
DEFINE FIELD IF NOT EXISTS content_hash ON memory_backup TYPE string;
DEFINE FIELD IF NOT EXISTS operation    ON memory_backup TYPE string
  ASSERT $value IN ['update', 'delete', 'edit', 'append', 'prepend'];
DEFINE FIELD IF NOT EXISTS source_agent ON memory_backup TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at   ON memory_backup TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS backup_entry_id   ON memory_backup FIELDS entry_id;
DEFINE INDEX IF NOT EXISTS backup_created    ON memory_backup FIELDS created_at;
DEFINE INDEX IF NOT EXISTS backup_entry_time ON memory_backup FIELDS entry_id, created_at;

-- =============================================================================
-- EMBEDDING CHUNKS (Issue #346)
-- =============================================================================
-- Chunked embeddings for long entries (>400 tokens). Each chunk is embedded
-- separately so semantic search covers the full content.

DEFINE TABLE IF NOT EXISTS embedding_chunk SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS entry_id ON embedding_chunk TYPE string;
DEFINE FIELD IF NOT EXISTS chunk_index ON embedding_chunk TYPE int;
DEFINE FIELD IF NOT EXISTS chunk_text ON embedding_chunk TYPE string;
DEFINE FIELD IF NOT EXISTS token_offset ON embedding_chunk TYPE int;
DEFINE FIELD IF NOT EXISTS token_count ON embedding_chunk TYPE int;
DEFINE FIELD IF NOT EXISTS embedding ON embedding_chunk TYPE array<float>;
DEFINE FIELD IF NOT EXISTS embedding_model ON embedding_chunk TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON embedding_chunk TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS chunk_entry_id ON embedding_chunk FIELDS entry_id;
DEFINE INDEX IF NOT EXISTS chunk_entry_idx ON embedding_chunk FIELDS entry_id, chunk_index UNIQUE;

-- =============================================================================
-- METADATA TABLES
-- =============================================================================

-- Deletion tombstones (for sync scenarios)
DEFINE TABLE IF NOT EXISTS deletion SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS deleted_at ON deletion TYPE datetime DEFAULT time::now();

-- Wake ritual sessions (server-side state, replaces signed client token)
DEFINE TABLE IF NOT EXISTS wake_session SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS bloom_ids ON wake_session TYPE array<string>;
DEFINE FIELD IF NOT EXISTS current_index ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS attempts_on_current ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS remembered_count ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS needed_help_count ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS skipped_count ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON wake_session TYPE datetime DEFAULT time::now();
-- mx#211: chunking cursor + monotonic step counter. current_chunk_index
-- advances within a chunked bloom before rolling over to the next bloom.
-- step is the token-signing counter — monotonic across all chunk advances
-- so tokens survive mid-ritual bloom re-chunking.
DEFINE FIELD IF NOT EXISTS current_chunk_index ON wake_session TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS step ON wake_session TYPE int DEFAULT 0;
-- Per-bloom phrase-count metadata (authored_phrase_count + is_phraseless).
-- `flexible` so the object shape can evolve without schema migration churn.
DEFINE FIELD IF NOT EXISTS bloom_chunk_meta ON wake_session FLEXIBLE TYPE array<object> DEFAULT [];
-- Legacy field from the pre-chunking design. Kept in the schema for
-- backward read compatibility during rollout; no longer written. SurrealDB
-- will keep any existing values but new sessions leave this empty.
DEFINE FIELD IF NOT EXISTS selected_phrase_indices ON wake_session TYPE array<int> DEFAULT [];
DEFINE FIELD OVERWRITE selected_phrase_indices ON wake_session TYPE array<int> DEFAULT [];
DEFINE INDEX IF NOT EXISTS wake_session_created ON wake_session FIELDS created_at;

-- =============================================================================
-- SEED DATA
-- =============================================================================

-- Categories
UPSERT category:pattern SET description = 'Reusable design patterns and solutions';
UPSERT category:technique SET description = 'Implementation techniques and approaches';
UPSERT category:insight SET description = 'Learnings and observations';
UPSERT category:gotcha SET description = 'Pitfalls and things to watch for';
UPSERT category:reference SET description = 'Reference material and documentation';
UPSERT category:decision SET description = 'Architectural and design decisions';
UPSERT category:bloom SET description = 'Identity discoveries and personal growth - Q private';
UPSERT category:session SET description = 'Daily session logs and activity records';

-- Source types
UPSERT source_type:manual SET description = 'Manually entered by human';
UPSERT source_type:ram SET description = 'Promoted from agent RAM';
UPSERT source_type:cache SET description = 'Extracted from workflow cache';
UPSERT source_type:agent_session SET description = 'Created during agent session';

-- Entry types
UPSERT entry_type:primary SET description = 'Original knowledge entry';
UPSERT entry_type:summary SET description = 'Summarized from other content';
UPSERT entry_type:synthesis SET description = 'Combined from multiple sources';

-- Content types
UPSERT content_type:text SET description = 'Plain text or markdown', file_extensions = 'md,txt,text';
UPSERT content_type:code SET description = 'Source code or scripts', file_extensions = 'py,rs,js,ts,sh,bash,rb,go,java,c,cpp,h';
UPSERT content_type:config SET description = 'Configuration files', file_extensions = 'json,yaml,yml,toml,xml,ini,env';
UPSERT content_type:data SET description = 'Data files or fixtures', file_extensions = 'json,csv,sql,fiche,schema';
UPSERT content_type:binary SET description = 'Binary or encoded content', file_extensions = 'bin,dat,b64';

-- Relationship types
UPSERT relationship_type:related SET description = 'General relationship', directional = false;
UPSERT relationship_type:supersedes SET description = 'Replaces older entry', directional = true;
UPSERT relationship_type:extends SET description = 'Builds upon', directional = true;
UPSERT relationship_type:implements SET description = 'Implements concept', directional = true;
UPSERT relationship_type:contradicts SET description = 'Conflicts with', directional = false;
UPSERT relationship_type:example_of SET description = 'Is an example of', directional = true;

-- Session types
UPSERT session_type:claude_desktop SET description = 'Claude Desktop app session';
UPSERT session_type:claude_code SET description = 'Claude Code CLI session';
UPSERT session_type:agent_task SET description = 'Dispatched agent task';
UPSERT session_type:manual SET description = 'Manual CLI interaction';

-- Common applicability types
UPSERT applicability_type:cross_platform SET description = 'Works on all platforms', scope = 'platform';
UPSERT applicability_type:linux SET description = 'Linux-specific', scope = 'platform';
UPSERT applicability_type:macos SET description = 'macOS-specific', scope = 'platform';
UPSERT applicability_type:windows SET description = 'Windows-specific', scope = 'platform';
UPSERT applicability_type:rust SET description = 'Rust language', scope = 'language';
UPSERT applicability_type:python SET description = 'Python language', scope = 'language';
UPSERT applicability_type:typescript SET description = 'TypeScript/JavaScript', scope = 'language';
UPSERT applicability_type:shell SET description = 'Shell scripting', scope = 'language';
UPSERT applicability_type:async SET description = 'Async/concurrent patterns', scope = 'domain';
UPSERT applicability_type:cli SET description = 'Command-line tools', scope = 'domain';
UPSERT applicability_type:api SET description = 'API design', scope = 'domain';

-- =============================================================================
-- COLD-UPGRADE BACKFILL (Issues #72, #73, #122, #352, #360 + resonance cascade)
-- =============================================================================
--
-- WHY THIS SECTION EXISTS
-- -----------------------
-- Our tables are SCHEMAFULL. SurrealDB validates the ENTIRE record on ANY
-- write — both a full UPSERT and a partial `SET`. A `DEFAULT` clause on a
-- field only fires when a NEW record is created; it does NOT fire when an
-- existing field is re-DEFINEd. So when we add a new REQUIRED (non-`option<>`)
-- field to an existing table, every pre-existing row is left with that field
-- as NONE. Reads survive via projection coalescing (e.g. the
-- `IF chunk_count THEN chunk_count ELSE 0 END` projection — see
-- knowledge_select_fields() in src/surreal_db/knowledge.rs), but the next WRITE
-- that touches such a row throws
-- `Found NONE for field X ... but expected a <type>`.
--
-- ISSUE #360 — THE COLD-UPGRADE TRAP (why this whole section was rewritten)
-- ------------------------------------------------------------------------
-- A `DEFINE FIELD ... TYPE <strict>` does NOT validate at define-time. The
-- strict type is enforced only on the next WRITE touching a row. So the OLD
-- ordering (strict DEFINE above, then a bare `UPDATE ... WHERE X IS NONE`
-- backfill here) does not actually work on a populated pre-existing graph:
--   * the backfill UPDATE is ITSELF the first write touching legacy rows, and
--   * a SCHEMAFULL write validates EVERY required field at once — so the very
--     first backfill UPDATE (historically the wake_phrases migration) aborts
--     because triggers/chunk_count/format AND the resonance-cascade ints
--     (resonance/activation_count/decay_rate) are still NONE on that same row.
--   * apply_schema sends the whole file through one `db.query` and uses
--     `take_errors()` (all-or-nothing), so one stranded row fails every connect.
-- Reported by the hearth federation (Soren/Geoff) on a ~5,283-row graph; the
-- interim workaround was MX_SKIP_SCHEMA=1.
--
-- WHAT DOESN'T WORK (do not relitigate — all tried, all fail):
--   * DEFAULT [] / DEFAULT 0 — fires only on create, never backfills.
--   * backfill placed AFTER the strict DEFINE — the backfill is the write that
--     dies.
--   * backfill placed BEFORE the DEFINE on a SCHEMAFULL table — `SET` of an
--     undefined field is SILENTLY DROPPED, so the row is never conformed.
--   * option<>-first then tighten with `IF NOT EXISTS` — the tighten is SKIPPED
--     because the field already exists, leaving it loose forever.
--
-- THE WORKING SEQUENCE (option → backfill → OVERWRITE-tighten):
--   1. Each required, legacy-added field is LOOSENED to `option<...>` at its
--      site above — the full set is triggers, wake_phrases, chunk_count, format,
--      resonance, activation_count, decay_rate (every REQUIRED knowledge field
--      that the read path coalesces in knowledge_select_fields(), i.e. every one
--      that can be NONE on a legacy row) — with `DEFINE FIELD OVERWRITE`:
--      OVERWRITE, not IF NOT EXISTS, because on an upgrade the field already
--      exists as the strict type and IF NOT EXISTS would skip, leaving it
--      strict. Loosening lets a write touch the row while the field is NONE.
--   2. Backfill ALL of them here, while every one is still loose. Because all
--      seven are option<> at this point, these UPDATEs pass whole-record
--      validation even though the others have not been set yet on that row.
--      ORDERING INVARIANT: every strand-hazard field is loosened (step 1, above
--      the SEED DATA) before ANY write touches a knowledge row — the seed
--      UPSERTs write only lookup tables, and the first knowledge-row write is
--      the backfill block here, by which point all seven are loose.
--   3. THEN tighten each back to its strict type with `DEFINE FIELD OVERWRITE`
--      (NOT `IF NOT EXISTS`, which would skip). After the backfill no row holds
--      NONE, so the strict type — enforced on the next write — is satisfied.
--   4. Define `triggers`' element index last, once `triggers` is strict again.
--
-- This file is replayed on EVERY connection (embedded AND network — see
-- connection.rs apply_schema) and on every `mx migrate`. IDEMPOTENCY: the
-- `WHERE <f> IS NONE` backfills are no-ops on a healthy graph (nothing is NONE),
-- and `DEFINE FIELD OVERWRITE` re-applies the same definition cleanly. A second
-- full apply succeeds and changes nothing.
--
-- THE RULE (read this before adding a required field)
-- ---------------------------------------------------
-- Whenever you add a new REQUIRED (non-`option<>`) field to an existing table,
-- DO NOT just write `DEFINE FIELD ... TYPE <strict> DEFAULT <d>` above. Use the
-- three-step cold-upgrade pattern: `DEFINE FIELD OVERWRITE` the field as
-- `option<>` at its site (OVERWRITE so an upgrade from a strict prior version
-- actually loosens), then add a `WHERE <field> IS NONE` backfill in THIS block,
-- then a paired `DEFINE FIELD OVERWRITE ... TYPE <strict> DEFAULT <d>` tighten
-- here. Never leave a field permanently `option<>` just to dodge this —
-- downstream code assumes presence; backfill + tighten is the correct fix. If
-- the backfill COMPUTES a value (not a constant), add a test that exercises the
-- non-empty case — assert the computed value matches the real count, not just
-- that the statement runs. Issue #352 is the cautionary tale: a chunk_count backfill
-- shipped relying on a non-obvious SurrealDB sub-select quirk with only a
-- throwaway probe; the permanent test (test_backfill_chunk_count_* in
-- src/surreal_db/tests.rs) now guards it. Issue #360 is the second cautionary
-- tale: the end-to-end apply_schema replay test (test_cold_upgrade_* in the
-- same file) now guards the ordering itself.

-- STEP 2 — backfill every legacy-added required field while all are still
-- option<> (so each UPDATE passes whole-record validation regardless of the
-- others still being NONE on the same row).

-- Issues #72/#73: migrate single wake_phrase to wake_phrases array.
-- Guard: only rows where wake_phrases was never set.
UPDATE knowledge SET wake_phrases = IF wake_phrase != NONE THEN [wake_phrase] ELSE [] END
WHERE wake_phrases IS NONE;

-- Issue #246: triggers has no legacy value to recover — default empty.
UPDATE knowledge SET triggers = [] WHERE triggers IS NONE;

-- Issue #122: backfill format field for existing entries.
UPDATE knowledge SET format = 'markdown' WHERE format IS NONE;

-- Resonance cascade fields: legacy rows that predate the wake-up cascade hold
-- NONE for these required ints/float. Backfill to their DEFAULTs while loose.
-- (Same #360 hazard as the four above — see the field-site comment.)
UPDATE knowledge SET resonance = 0 WHERE resonance IS NONE;
UPDATE knowledge SET activation_count = 0 WHERE activation_count IS NONE;
UPDATE knowledge SET decay_rate = 0.0 WHERE decay_rate IS NONE;

-- Issue #352: backfill chunk_count for entries that predate chunking (Issue #346).
-- chunk_count is `int DEFAULT 0` (required); rows that predate it are stranded
-- with NONE — broken on next write.
--
-- The correct value is the actual number of embedding_chunk rows for the entry.
-- ID MISMATCH: a knowledge record id is `knowledge:<hex>`, but
-- `embedding_chunk.entry_id` stores the `kn-<hex>` form (e.g. kn-<hex>) as a
-- plain string — NOT string-identical. We must translate the prefix when
-- joining: build the `kn-<hex>` key from meta::id(id) and count matching chunks.
-- For the pre-chunking rows this correctly yields 0; for genuinely chunked
-- entries it yields the real count. Guard `WHERE chunk_count IS NONE` keeps it a
-- no-op once applied.
--
-- We use `array::len(SELECT id FROM ...)` rather than a `count() ... GROUP ALL`
-- sub-select: it returns a plain int (naturally 0 on no matches), so there is no
-- `[0].count ?? 0` projection-shape dependency to break across SurrealDB
-- versions. Verified empirically against the live DB (2.6.1): both forms yield
-- identical per-entry counts (24 chunked entries, 64 chunks total over 368 rows).
-- Regression coverage: see test_backfill_chunk_count_* in src/surreal_db/tests.rs.
UPDATE knowledge SET chunk_count = array::len(
    SELECT id FROM embedding_chunk
    WHERE entry_id = string::concat('kn-', meta::id($parent.id))
)
WHERE chunk_count IS NONE;

-- STEP 3 — tighten each field back to its strict required type now that no row
-- holds NONE. MUST be OVERWRITE: IF NOT EXISTS would silently skip and leave
-- the field loose (option<>) forever (Issue #360).
DEFINE FIELD OVERWRITE wake_phrases ON knowledge TYPE array<string> DEFAULT [];
DEFINE FIELD OVERWRITE triggers ON knowledge TYPE array<string> DEFAULT [];
DEFINE FIELD OVERWRITE format ON knowledge TYPE string DEFAULT 'markdown'
  ASSERT $value IN ['markdown', 'json', 'stele:markdown', 'stele:ascii', 'stele:light', 'stele:full'];
DEFINE FIELD OVERWRITE chunk_count ON knowledge TYPE int DEFAULT 0;
-- Resonance cascade fields (legacy-added, Issue #360 cohort): tighten back to
-- their strict required types now that no row holds NONE.
DEFINE FIELD OVERWRITE resonance ON knowledge TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE activation_count ON knowledge TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE decay_rate ON knowledge TYPE float DEFAULT 0.0;

-- STEP 4 — define the triggers element index now that `triggers` is strict
-- `array<string>` again.
-- INDEX HONESTY (Issue #246, PR3): this is a per-ELEMENT index over the array
-- contents. It accelerates equality/CONTAINS lookups (e.g.
-- `WHERE triggers CONTAINS 'brad'`), NOT the `array::len(triggers ?? []) > 0`
-- non-emptiness prefilter that `mx memory trigger-check` currently uses (that
-- resolves via full scan regardless). It is kept deliberately: the actual
-- matching is word-boundary + stemmed and happens in Rust (it cannot be pushed
-- into a SurrealDB index), but a future hook-side fast path / "which memory owns
-- trigger X" lookup will want CONTAINS, which this index serves. If those never
-- materialize, this index is a candidate for removal.
DEFINE INDEX IF NOT EXISTS knowledge_triggers ON knowledge FIELDS triggers;
