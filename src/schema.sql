-- Zion v3 Schema - Fully Normalized Knowledge Database
-- All categorical fields use lookup tables
-- All many-to-many relationships use junction tables

-- ============================================================================
-- LOOKUP TABLES
-- ============================================================================

-- Categories (pattern, technique, insight, etc.)
CREATE TABLE IF NOT EXISTS categories (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Projects (dotmatrix, mx, base-d, etc.)
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,               -- Slug: dotmatrix, mx, base-d
    name TEXT NOT NULL,                -- Display name: "dotmatrix", "MX CLI"
    path TEXT,                         -- File path: ~/work/personal/code/mx
    repo_url TEXT,                     -- GitHub URL
    description TEXT,                  -- What this project is
    active INTEGER DEFAULT 1,          -- Boolean: currently maintained
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Applicability types (cross-platform, rust-specific, etc.)
CREATE TABLE IF NOT EXISTS applicability_types (
    id TEXT PRIMARY KEY,               -- rust, python, cross-platform, linux-only
    description TEXT NOT NULL,         -- When/where this applies
    scope TEXT,                        -- language, platform, tool, domain
    created_at TEXT NOT NULL
);

-- Source types (manual, ram, cache, agent_session)
CREATE TABLE IF NOT EXISTS source_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Entry types (primary, summary, synthesis)
CREATE TABLE IF NOT EXISTS entry_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Relationship types (related, supersedes, extends, etc.)
CREATE TABLE IF NOT EXISTS relationship_types (
    id TEXT PRIMARY KEY,               -- related, supersedes, extends, implements
    description TEXT NOT NULL,
    directional INTEGER DEFAULT 1,     -- Boolean: does A->B differ from B->A?
    created_at TEXT NOT NULL
);

-- Session types (claude_desktop, agent_task, manual)
CREATE TABLE IF NOT EXISTS session_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Agents (neo, smith, trinity, etc.)
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    description TEXT,
    domain TEXT,
    created_at TEXT,
    updated_at TEXT
);

-- ============================================================================
-- CORE TABLES
-- ============================================================================

-- Sessions (with FK to session_types)
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,               -- Session UUID or hash
    session_type_id TEXT NOT NULL,     -- FK to session_types
    project_id TEXT,                   -- FK to projects (nullable)
    started_at TEXT NOT NULL,
    ended_at TEXT,
    metadata TEXT,                     -- JSON: additional context

    FOREIGN KEY (session_type_id) REFERENCES session_types(id),
    FOREIGN KEY (project_id) REFERENCES projects(id)
);

-- Knowledge entries (all FKs, no hardcoded strings)
CREATE TABLE IF NOT EXISTS knowledge (
    id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL,         -- FK to categories
    title TEXT NOT NULL,
    body TEXT,
    summary TEXT,
    source_project_id TEXT,            -- FK to projects (nullable)
    source_agent_id TEXT,              -- FK to agents (nullable)
    file_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,

    -- Provenance
    source_type_id TEXT NOT NULL,      -- FK to source_types
    entry_type_id TEXT NOT NULL,       -- FK to entry_types
    session_id TEXT,                   -- FK to sessions (nullable)
    ephemeral INTEGER DEFAULT 0,       -- Boolean: may be pruned

    FOREIGN KEY (category_id) REFERENCES categories(id),
    FOREIGN KEY (source_project_id) REFERENCES projects(id),
    FOREIGN KEY (source_agent_id) REFERENCES agents(id),
    FOREIGN KEY (source_type_id) REFERENCES source_types(id),
    FOREIGN KEY (entry_type_id) REFERENCES entry_types(id),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- ============================================================================
-- JUNCTION TABLES (Many-to-Many Relationships)
-- ============================================================================

-- Knowledge can have multiple applicability contexts
CREATE TABLE IF NOT EXISTS knowledge_applicability (
    entry_id TEXT NOT NULL,
    applicability_id TEXT NOT NULL,
    PRIMARY KEY (entry_id, applicability_id),
    FOREIGN KEY (entry_id) REFERENCES knowledge(id) ON DELETE CASCADE,
    FOREIGN KEY (applicability_id) REFERENCES applicability_types(id)
);

-- Knowledge tags junction table
CREATE TABLE IF NOT EXISTS tags (
    entry_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (entry_id, tag),
    FOREIGN KEY (entry_id) REFERENCES knowledge(id) ON DELETE CASCADE
);

-- Relationships between knowledge entries (with FK to relationship_types)
CREATE TABLE IF NOT EXISTS relationships (
    id TEXT PRIMARY KEY,
    from_entry_id TEXT NOT NULL,
    to_entry_id TEXT NOT NULL,
    relationship_type TEXT NOT NULL,   -- FK to relationship_types
    created_at TEXT NOT NULL,
    FOREIGN KEY (from_entry_id) REFERENCES knowledge(id) ON DELETE CASCADE,
    FOREIGN KEY (to_entry_id) REFERENCES knowledge(id) ON DELETE CASCADE,
    FOREIGN KEY (relationship_type) REFERENCES relationship_types(id),
    UNIQUE (from_entry_id, to_entry_id, relationship_type)
);

-- Project applicability (many-to-many: projects <-> applicability_types)
CREATE TABLE IF NOT EXISTS project_applicability (
    project_id TEXT NOT NULL,
    applicability_id TEXT NOT NULL,
    PRIMARY KEY (project_id, applicability_id),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (applicability_id) REFERENCES applicability_types(id)
);

-- Project tags (many-to-many: projects <-> freeform tags)
CREATE TABLE IF NOT EXISTS project_tags (
    project_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (project_id, tag),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- ============================================================================
-- METADATA TABLES
-- ============================================================================

-- Deletions tombstones
CREATE TABLE IF NOT EXISTS deletions (
    id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL
);

-- ============================================================================
-- INDEXES
-- ============================================================================

-- Knowledge indexes
CREATE INDEX IF NOT EXISTS idx_knowledge_category ON knowledge(category_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_project ON knowledge(source_project_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_agent ON knowledge(source_agent_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_type ON knowledge(source_type_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_entry_type ON knowledge(entry_type_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_session ON knowledge(session_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_updated ON knowledge(updated_at);

-- Knowledge junction table indexes
CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
CREATE INDEX IF NOT EXISTS idx_applicability_entry ON knowledge_applicability(entry_id);
CREATE INDEX IF NOT EXISTS idx_applicability_type ON knowledge_applicability(applicability_id);

-- Project junction table indexes
CREATE INDEX IF NOT EXISTS idx_project_applicability_project ON project_applicability(project_id);
CREATE INDEX IF NOT EXISTS idx_project_applicability_type ON project_applicability(applicability_id);
CREATE INDEX IF NOT EXISTS idx_project_tags_project ON project_tags(project_id);
CREATE INDEX IF NOT EXISTS idx_project_tags_tag ON project_tags(tag);

-- Lookup table indexes
CREATE INDEX IF NOT EXISTS idx_agents_domain ON agents(domain);
CREATE INDEX IF NOT EXISTS idx_sessions_type ON sessions(session_type_id);
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_relationships_type ON relationships(relationship_type);
CREATE INDEX IF NOT EXISTS idx_relationships_from ON relationships(from_entry_id);
CREATE INDEX IF NOT EXISTS idx_relationships_to ON relationships(to_entry_id);
CREATE INDEX IF NOT EXISTS idx_applicability_scope ON applicability_types(scope);
CREATE INDEX IF NOT EXISTS idx_projects_active ON projects(active);
