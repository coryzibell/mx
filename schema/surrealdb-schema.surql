-- =============================================================================
-- Memory Knowledge Base - SurrealDB Schema
-- Version: 1.0
-- Migrated from SQLite v5
-- =============================================================================

-- Use namespace and database
-- Note: Executed at connection time in Rust, not in schema file

-- =============================================================================
-- LOOKUP TABLES (Records with metadata)
-- =============================================================================

-- Categories: pattern, technique, insight, etc.
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

-- Resonance fields for wake-up cascade
DEFINE FIELD IF NOT EXISTS resonance ON knowledge TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS resonance_type ON knowledge TYPE option<string>
  ASSERT $value IN [null, 'foundational', 'transformative', 'relational', 'operational', 'ephemeral'];
DEFINE FIELD IF NOT EXISTS last_activated ON knowledge TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS activation_count ON knowledge TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS decay_rate ON knowledge TYPE float DEFAULT 0.0;
DEFINE FIELD IF NOT EXISTS anchors ON knowledge TYPE option<array<string>>;

-- Issue #72: Multiple wake phrases for ritual variety
DEFINE FIELD IF NOT EXISTS wake_phrases ON knowledge TYPE array<string> DEFAULT [];

-- Issue #73: Custom wake order (separate from resonance)
DEFINE FIELD IF NOT EXISTS wake_order ON knowledge TYPE option<int>;
DEFINE INDEX IF NOT EXISTS knowledge_wake_order ON knowledge FIELDS wake_order;

-- DEPRECATED: Kept for backward compatibility during migration
DEFINE FIELD IF NOT EXISTS wake_phrase ON knowledge TYPE option<string>;

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

-- Custom analyzer for knowledge search
DEFINE ANALYZER IF NOT EXISTS memory_analyzer
    TOKENIZERS class
    FILTERS lowercase, snowball(english);

-- Search indexes
DEFINE INDEX IF NOT EXISTS knowledge_title_fts ON knowledge
    FIELDS title SEARCH ANALYZER memory_analyzer BM25;
DEFINE INDEX IF NOT EXISTS knowledge_body_fts ON knowledge
    FIELDS body SEARCH ANALYZER memory_analyzer BM25;
DEFINE INDEX IF NOT EXISTS knowledge_summary_fts ON knowledge
    FIELDS summary SEARCH ANALYZER memory_analyzer BM25;

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
-- METADATA TABLES
-- =============================================================================

-- Deletion tombstones (for sync scenarios)
DEFINE TABLE IF NOT EXISTS deletion SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS deleted_at ON deletion TYPE datetime DEFAULT time::now();

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
-- DATA MIGRATION (Issues #72, #73)
-- =============================================================================

-- Migrate existing single wake_phrase to wake_phrases array
-- Handle records where wake_phrases is NONE (not yet created)
UPDATE knowledge SET wake_phrases = IF wake_phrase != NONE THEN [wake_phrase] ELSE [] END
WHERE wake_phrases IS NONE;
