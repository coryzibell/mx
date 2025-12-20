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
DEFINE TABLE category SCHEMAFULL;
DEFINE FIELD description ON category TYPE string;
DEFINE FIELD created_at ON category TYPE datetime DEFAULT time::now();

-- Projects: dotmatrix, mx, base-d, etc.
DEFINE TABLE project SCHEMAFULL;
DEFINE FIELD name ON project TYPE string;
DEFINE FIELD path ON project TYPE option<string>;
DEFINE FIELD repo_url ON project TYPE option<string>;
DEFINE FIELD description ON project TYPE option<string>;
DEFINE FIELD active ON project TYPE bool DEFAULT true;
DEFINE FIELD created_at ON project TYPE datetime DEFAULT time::now();
DEFINE FIELD updated_at ON project TYPE datetime DEFAULT time::now();
DEFINE INDEX project_active ON project FIELDS active;

-- Agents: neo, smith, trinity, etc.
DEFINE TABLE agent SCHEMAFULL;
DEFINE FIELD description ON agent TYPE option<string>;
DEFINE FIELD domain ON agent TYPE option<string>;
DEFINE FIELD created_at ON agent TYPE datetime DEFAULT time::now();
DEFINE FIELD updated_at ON agent TYPE datetime DEFAULT time::now();
DEFINE INDEX agent_domain ON agent FIELDS domain;

-- Applicability types: rust, python, cross-platform, linux-only
DEFINE TABLE applicability_type SCHEMAFULL;
DEFINE FIELD description ON applicability_type TYPE string;
DEFINE FIELD scope ON applicability_type TYPE option<string>;  -- language, platform, tool, domain
DEFINE FIELD created_at ON applicability_type TYPE datetime DEFAULT time::now();
DEFINE INDEX applicability_scope ON applicability_type FIELDS scope;

-- Source types: manual, ram, cache, agent_session
DEFINE TABLE source_type SCHEMAFULL;
DEFINE FIELD description ON source_type TYPE string;
DEFINE FIELD created_at ON source_type TYPE datetime DEFAULT time::now();

-- Entry types: primary, summary, synthesis
DEFINE TABLE entry_type SCHEMAFULL;
DEFINE FIELD description ON entry_type TYPE string;
DEFINE FIELD created_at ON entry_type TYPE datetime DEFAULT time::now();

-- Content types: text, code, config, data, binary
DEFINE TABLE content_type SCHEMAFULL;
DEFINE FIELD description ON content_type TYPE string;
DEFINE FIELD file_extensions ON content_type TYPE option<string>;  -- comma-separated
DEFINE FIELD created_at ON content_type TYPE datetime DEFAULT time::now();

-- Relationship types: related, supersedes, extends, implements
DEFINE TABLE relationship_type SCHEMAFULL;
DEFINE FIELD description ON relationship_type TYPE string;
DEFINE FIELD directional ON relationship_type TYPE bool DEFAULT true;
DEFINE FIELD created_at ON relationship_type TYPE datetime DEFAULT time::now();

-- Session types: claude_desktop, agent_task, manual
DEFINE TABLE session_type SCHEMAFULL;
DEFINE FIELD description ON session_type TYPE string;
DEFINE FIELD created_at ON session_type TYPE datetime DEFAULT time::now();

-- Tags: freeform labels
DEFINE TABLE tag SCHEMAFULL;
DEFINE FIELD name ON tag TYPE string;
DEFINE FIELD created_at ON tag TYPE datetime DEFAULT time::now();
DEFINE INDEX tag_name ON tag FIELDS name UNIQUE;

-- =============================================================================
-- CORE TABLES
-- =============================================================================

-- Sessions
DEFINE TABLE session SCHEMAFULL;
DEFINE FIELD session_type ON session TYPE record<session_type>;
DEFINE FIELD project ON session TYPE option<record<project>>;
DEFINE FIELD started_at ON session TYPE datetime DEFAULT time::now();
DEFINE FIELD ended_at ON session TYPE option<datetime>;
DEFINE FIELD metadata ON session TYPE option<object>;  -- flexible JSON
DEFINE INDEX session_type_idx ON session FIELDS session_type;
DEFINE INDEX session_project_idx ON session FIELDS project;

-- Knowledge entries (main entity)
DEFINE TABLE knowledge SCHEMAFULL;
DEFINE FIELD title ON knowledge TYPE string;
DEFINE FIELD body ON knowledge TYPE option<string>;
DEFINE FIELD summary ON knowledge TYPE option<string>;
DEFINE FIELD file_path ON knowledge TYPE option<string>;
DEFINE FIELD content_hash ON knowledge TYPE string;
DEFINE FIELD ephemeral ON knowledge TYPE bool DEFAULT false;
DEFINE FIELD created_at ON knowledge TYPE datetime DEFAULT time::now();
DEFINE FIELD updated_at ON knowledge TYPE datetime DEFAULT time::now();

-- Record links (replaces foreign keys)
DEFINE FIELD category ON knowledge TYPE record<category>;
DEFINE FIELD source_project ON knowledge TYPE option<record<project>>;
DEFINE FIELD source_agent ON knowledge TYPE option<record<agent>>;
DEFINE FIELD source_type ON knowledge TYPE record<source_type>;
DEFINE FIELD entry_type ON knowledge TYPE record<entry_type>;
DEFINE FIELD content_type ON knowledge TYPE record<content_type> DEFAULT content_type:text;
DEFINE FIELD session ON knowledge TYPE option<record<session>>;

-- Indexes for common queries
DEFINE INDEX knowledge_category ON knowledge FIELDS category;
DEFINE INDEX knowledge_source_project ON knowledge FIELDS source_project;
DEFINE INDEX knowledge_source_agent ON knowledge FIELDS source_agent;
DEFINE INDEX knowledge_source_type ON knowledge FIELDS source_type;
DEFINE INDEX knowledge_entry_type ON knowledge FIELDS entry_type;
DEFINE INDEX knowledge_content_type ON knowledge FIELDS content_type;
DEFINE INDEX knowledge_session ON knowledge FIELDS session;
DEFINE INDEX knowledge_updated ON knowledge FIELDS updated_at;
DEFINE INDEX knowledge_ephemeral ON knowledge FIELDS ephemeral;

-- =============================================================================
-- FULL-TEXT SEARCH
-- =============================================================================

-- Custom analyzer for knowledge search
DEFINE ANALYZER memory_analyzer
    TOKENIZERS class
    FILTERS lowercase, snowball(english);

-- Search indexes
DEFINE INDEX knowledge_title_fts ON knowledge
    FIELDS title SEARCH ANALYZER memory_analyzer BM25;
DEFINE INDEX knowledge_body_fts ON knowledge
    FIELDS body SEARCH ANALYZER memory_analyzer BM25;
DEFINE INDEX knowledge_summary_fts ON knowledge
    FIELDS summary SEARCH ANALYZER memory_analyzer BM25;

-- =============================================================================
-- GRAPH RELATIONS (Replace junction tables)
-- =============================================================================

-- Knowledge <-> Tag relationship
DEFINE TABLE tagged_with SCHEMAFULL TYPE RELATION
    FROM knowledge TO tag;
DEFINE FIELD created_at ON tagged_with TYPE datetime DEFAULT time::now();
-- Prevent duplicate edges
DEFINE INDEX tagged_with_unique ON tagged_with FIELDS in, out UNIQUE;

-- Knowledge <-> Applicability relationship
DEFINE TABLE applies_to SCHEMAFULL TYPE RELATION
    FROM knowledge TO applicability_type;
DEFINE FIELD created_at ON applies_to TYPE datetime DEFAULT time::now();
DEFINE INDEX applies_to_unique ON applies_to FIELDS in, out UNIQUE;

-- Project <-> Applicability relationship
DEFINE TABLE project_applies_to SCHEMAFULL TYPE RELATION
    FROM project TO applicability_type;
DEFINE FIELD created_at ON project_applies_to TYPE datetime DEFAULT time::now();
DEFINE INDEX project_applies_unique ON project_applies_to FIELDS in, out UNIQUE;

-- Project <-> Tag relationship
DEFINE TABLE project_tagged_with SCHEMAFULL TYPE RELATION
    FROM project TO tag;
DEFINE FIELD created_at ON project_tagged_with TYPE datetime DEFAULT time::now();
DEFINE INDEX project_tagged_unique ON project_tagged_with FIELDS in, out UNIQUE;

-- Knowledge <-> Knowledge relationship (the graph!)
DEFINE TABLE relates_to SCHEMAFULL TYPE RELATION
    FROM knowledge TO knowledge;
DEFINE FIELD relationship_type ON relates_to TYPE record<relationship_type>;
DEFINE FIELD created_at ON relates_to TYPE datetime DEFAULT time::now();
-- Unique constraint on the triple (from, to, type)
DEFINE INDEX relates_to_unique ON relates_to FIELDS in, out, relationship_type UNIQUE;

-- =============================================================================
-- METADATA TABLES
-- =============================================================================

-- Deletion tombstones (for sync scenarios)
DEFINE TABLE deletion SCHEMAFULL;
DEFINE FIELD deleted_at ON deletion TYPE datetime DEFAULT time::now();

-- =============================================================================
-- SEED DATA
-- =============================================================================

-- Categories
CREATE category:pattern SET description = 'Reusable design patterns and solutions';
CREATE category:technique SET description = 'Implementation techniques and approaches';
CREATE category:insight SET description = 'Learnings and observations';
CREATE category:gotcha SET description = 'Pitfalls and things to watch for';
CREATE category:reference SET description = 'Reference material and documentation';
CREATE category:decision SET description = 'Architectural and design decisions';

-- Source types
CREATE source_type:manual SET description = 'Manually entered by human';
CREATE source_type:ram SET description = 'Promoted from agent RAM';
CREATE source_type:cache SET description = 'Extracted from workflow cache';
CREATE source_type:agent_session SET description = 'Created during agent session';

-- Entry types
CREATE entry_type:primary SET description = 'Original knowledge entry';
CREATE entry_type:summary SET description = 'Summarized from other content';
CREATE entry_type:synthesis SET description = 'Combined from multiple sources';

-- Content types
CREATE content_type:text SET description = 'Plain text or markdown', file_extensions = 'md,txt,text';
CREATE content_type:code SET description = 'Source code or scripts', file_extensions = 'py,rs,js,ts,sh,bash,rb,go,java,c,cpp,h';
CREATE content_type:config SET description = 'Configuration files', file_extensions = 'json,yaml,yml,toml,xml,ini,env';
CREATE content_type:data SET description = 'Data files or fixtures', file_extensions = 'json,csv,sql,fiche,schema';
CREATE content_type:binary SET description = 'Binary or encoded content', file_extensions = 'bin,dat,b64';

-- Relationship types
CREATE relationship_type:related SET description = 'General relationship', directional = false;
CREATE relationship_type:supersedes SET description = 'Replaces older entry', directional = true;
CREATE relationship_type:extends SET description = 'Builds upon', directional = true;
CREATE relationship_type:implements SET description = 'Implements concept', directional = true;
CREATE relationship_type:contradicts SET description = 'Conflicts with', directional = false;
CREATE relationship_type:example_of SET description = 'Is an example of', directional = true;

-- Session types
CREATE session_type:claude_desktop SET description = 'Claude Desktop app session';
CREATE session_type:claude_code SET description = 'Claude Code CLI session';
CREATE session_type:agent_task SET description = 'Dispatched agent task';
CREATE session_type:manual SET description = 'Manual CLI interaction';

-- Common applicability types
CREATE applicability_type:cross_platform SET description = 'Works on all platforms', scope = 'platform';
CREATE applicability_type:linux SET description = 'Linux-specific', scope = 'platform';
CREATE applicability_type:macos SET description = 'macOS-specific', scope = 'platform';
CREATE applicability_type:windows SET description = 'Windows-specific', scope = 'platform';
CREATE applicability_type:rust SET description = 'Rust language', scope = 'language';
CREATE applicability_type:python SET description = 'Python language', scope = 'language';
CREATE applicability_type:typescript SET description = 'TypeScript/JavaScript', scope = 'language';
CREATE applicability_type:shell SET description = 'Shell scripting', scope = 'language';
CREATE applicability_type:async SET description = 'Async/concurrent patterns', scope = 'domain';
CREATE applicability_type:cli SET description = 'Command-line tools', scope = 'domain';
CREATE applicability_type:api SET description = 'API design', scope = 'domain';
