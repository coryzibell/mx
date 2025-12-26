-- Migration from v2 to v3
-- Eliminates ALL hardcoded enums, adds projects table, normalizes applicability
-- Safe to run multiple times (idempotent)

BEGIN TRANSACTION;

-- Step 1: Create new lookup tables
CREATE TABLE IF NOT EXISTS categories (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT,
    repo_url TEXT,
    description TEXT,
    active INTEGER DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS applicability_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    scope TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS entry_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS relationship_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    directional INTEGER DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Step 2: Seed lookup tables with known values
INSERT OR IGNORE INTO categories (id, description, created_at) VALUES
    ('archive', 'Verbatim source documents - never summarized', datetime('now')),
    ('pattern', 'Recurring structural solutions', datetime('now')),
    ('technique', 'Specific procedural approaches', datetime('now')),
    ('insight', 'Key realizations and understanding', datetime('now')),
    ('ritual', 'Habitual practices and workflows', datetime('now')),
    ('artifact', 'Tools, scripts, templates', datetime('now')),
    ('chronicle', 'Historical records and narratives', datetime('now')),
    ('project', 'Project-specific knowledge', datetime('now')),
    ('future', 'Ideas and plans', datetime('now')),
    ('session', 'Daily session logs and activity records', datetime('now'));

INSERT OR IGNORE INTO source_types (id, description, created_at) VALUES
    ('manual', 'Manually entered knowledge', datetime('now')),
    ('ram', 'Absorbed from agent RAM', datetime('now')),
    ('cache', 'Absorbed from workflow cache', datetime('now')),
    ('agent_session', 'Captured during agent execution', datetime('now'));

INSERT OR IGNORE INTO entry_types (id, description, created_at) VALUES
    ('primary', 'Original source material', datetime('now')),
    ('summary', 'Condensed summary of primary material', datetime('now')),
    ('synthesis', 'Combined insights from multiple sources', datetime('now'));

INSERT OR IGNORE INTO relationship_types (id, description, directional, created_at) VALUES
    ('related', 'General association', 0, datetime('now')),
    ('supersedes', 'Replaces or deprecates', 1, datetime('now')),
    ('extends', 'Builds upon or expands', 1, datetime('now')),
    ('implements', 'Concrete realization of', 1, datetime('now')),
    ('contradicts', 'Conflicts with', 0, datetime('now'));

INSERT OR IGNORE INTO session_types (id, description, created_at) VALUES
    ('claude_desktop', 'Claude Desktop session', datetime('now')),
    ('agent_task', 'Agent task execution', datetime('now')),
    ('manual', 'Manual entry session', datetime('now')),
    ('batch_import', 'Bulk import operation', datetime('now'));

-- Seed known applicability types from existing data
INSERT OR IGNORE INTO applicability_types (id, description, scope, created_at)
SELECT DISTINCT
    LOWER(REPLACE(applicability, ' ', '-')),
    applicability,
    'general',
    datetime('now')
FROM knowledge
WHERE applicability IS NOT NULL AND applicability != '';

-- Seed known projects from existing data
INSERT OR IGNORE INTO projects (id, name, path, repo_url, description, active, created_at, updated_at)
SELECT DISTINCT
    source_project,
    source_project,
    NULL,
    NULL,
    'Migrated from v2 schema',
    1,
    datetime('now'),
    datetime('now')
FROM knowledge
WHERE source_project IS NOT NULL AND source_project != '';

-- Add well-known projects (will be ignored if already exist from data)
INSERT OR IGNORE INTO projects (id, name, path, repo_url, description, active, created_at, updated_at) VALUES
    ('mx', 'MX CLI', '~/work/personal/code/mx', 'https://github.com/coryzibell/mx', 'Matrix CLI tool', 1, datetime('now'), datetime('now')),
    ('dotmatrix', 'dotmatrix', '~/.matrix', 'https://github.com/coryzibell/dotmatrix', 'Matrix configuration repository', 1, datetime('now'), datetime('now')),
    ('base-d', 'base-d', '~/work/personal/code/base-d', NULL, 'Base-d encoder/decoder', 1, datetime('now'), datetime('now'));

-- Create project junction tables
CREATE TABLE IF NOT EXISTS project_applicability (
    project_id TEXT NOT NULL,
    applicability_id TEXT NOT NULL,
    PRIMARY KEY (project_id, applicability_id),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (applicability_id) REFERENCES applicability_types(id)
);

CREATE TABLE IF NOT EXISTS project_tags (
    project_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (project_id, tag),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Seed well-known applicability types for seeding project links
INSERT OR IGNORE INTO applicability_types (id, description, scope, created_at) VALUES
    ('rust', 'Rust programming language', 'language', datetime('now')),
    ('cli', 'Command-line interface tools', 'domain', datetime('now')),
    ('cross-platform', 'Works on all platforms', 'platform', datetime('now')),
    ('encoding', 'Data encoding/decoding', 'domain', datetime('now')),
    ('compression', 'Data compression', 'domain', datetime('now'));

-- Seed project applicability for known projects
INSERT OR IGNORE INTO project_applicability (project_id, applicability_id) VALUES
    ('mx', 'rust'),
    ('mx', 'cli'),
    ('mx', 'cross-platform'),
    ('base-d', 'rust'),
    ('base-d', 'cli'),
    ('base-d', 'encoding'),
    ('base-d', 'compression');

-- Seed project tags for known projects
INSERT OR IGNORE INTO project_tags (project_id, tag) VALUES
    ('mx', 'tooling'),
    ('mx', 'matrix'),
    ('mx', 'knowledge-management'),
    ('dotmatrix', 'config'),
    ('dotmatrix', 'agents'),
    ('base-d', 'unicode'),
    ('base-d', 'hashing');

-- Step 3: Create new sessions table with FK
CREATE TABLE IF NOT EXISTS sessions_v3 (
    id TEXT PRIMARY KEY,
    session_type_id TEXT NOT NULL,
    project_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    metadata TEXT,
    FOREIGN KEY (session_type_id) REFERENCES session_types(id),
    FOREIGN KEY (project_id) REFERENCES projects(id)
);

-- Migrate existing sessions (only if sessions table exists in v2 - which it doesn't, so skip)
-- V2 schema did not have sessions table, so no migration needed

-- Step 4: Create new knowledge table with all FKs
CREATE TABLE IF NOT EXISTS knowledge_v3 (
    id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    summary TEXT,
    source_project_id TEXT,
    source_agent_id TEXT,
    file_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_type_id TEXT NOT NULL,
    entry_type_id TEXT NOT NULL,
    session_id TEXT,
    ephemeral INTEGER DEFAULT 0,

    FOREIGN KEY (category_id) REFERENCES categories(id),
    FOREIGN KEY (source_project_id) REFERENCES projects(id),
    FOREIGN KEY (source_agent_id) REFERENCES agents(id),
    FOREIGN KEY (source_type_id) REFERENCES source_types(id),
    FOREIGN KEY (entry_type_id) REFERENCES entry_types(id),
    FOREIGN KEY (session_id) REFERENCES sessions_v3(id)
);

-- Step 5: Migrate knowledge data
INSERT INTO knowledge_v3 (
    id, category_id, title, body, summary,
    source_project_id, source_agent_id, file_path,
    created_at, updated_at, content_hash,
    source_type_id, entry_type_id, session_id, ephemeral
)
SELECT
    id,
    category,  -- Old TEXT becomes category_id FK
    title,
    body,
    summary,
    source_project,  -- Old TEXT becomes source_project_id FK
    source_agent,    -- Old TEXT becomes source_agent_id FK
    file_path,
    COALESCE(created_at, datetime('now')),
    COALESCE(updated_at, datetime('now')),
    COALESCE(content_hash, ''),
    COALESCE(source_type, 'manual'),
    COALESCE(entry_type, 'primary'),
    session_id,
    COALESCE(ephemeral, 0)
FROM knowledge;

-- Step 6: Create applicability junction table
CREATE TABLE IF NOT EXISTS knowledge_applicability (
    entry_id TEXT NOT NULL,
    applicability_id TEXT NOT NULL,
    PRIMARY KEY (entry_id, applicability_id),
    FOREIGN KEY (entry_id) REFERENCES knowledge_v3(id) ON DELETE CASCADE,
    FOREIGN KEY (applicability_id) REFERENCES applicability_types(id)
);

-- Migrate old applicability TEXT to junction table
-- Split on comma if multiple values exist
INSERT INTO knowledge_applicability (entry_id, applicability_id)
SELECT
    id,
    LOWER(REPLACE(applicability, ' ', '-'))
FROM knowledge
WHERE applicability IS NOT NULL AND applicability != '';

-- Step 7: Update relationships table with FK
CREATE TABLE IF NOT EXISTS relationships_v3 (
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    rel_type_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_id, to_id, rel_type_id),
    FOREIGN KEY (from_id) REFERENCES knowledge_v3(id) ON DELETE CASCADE,
    FOREIGN KEY (to_id) REFERENCES knowledge_v3(id) ON DELETE CASCADE,
    FOREIGN KEY (rel_type_id) REFERENCES relationship_types(id)
);

-- Migrate existing relationships (only if relationships table exists in v2)
-- V2 schema did not have relationships table, so no migration needed

-- Step 8: Swap tables
DROP TABLE IF EXISTS knowledge;
ALTER TABLE knowledge_v3 RENAME TO knowledge;

DROP TABLE IF EXISTS sessions;
ALTER TABLE sessions_v3 RENAME TO sessions;

DROP TABLE IF EXISTS relationships;
ALTER TABLE relationships_v3 RENAME TO relationships;

-- Step 9: Recreate indexes
CREATE INDEX IF NOT EXISTS idx_knowledge_category ON knowledge(category_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_project ON knowledge(source_project_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_agent ON knowledge(source_agent_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_source_type ON knowledge(source_type_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_entry_type ON knowledge(entry_type_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_session ON knowledge(session_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_updated ON knowledge(updated_at);

CREATE INDEX IF NOT EXISTS idx_applicability_entry ON knowledge_applicability(entry_id);
CREATE INDEX IF NOT EXISTS idx_applicability_type ON knowledge_applicability(applicability_id);

CREATE INDEX IF NOT EXISTS idx_project_applicability_project ON project_applicability(project_id);
CREATE INDEX IF NOT EXISTS idx_project_applicability_type ON project_applicability(applicability_id);
CREATE INDEX IF NOT EXISTS idx_project_tags_project ON project_tags(project_id);
CREATE INDEX IF NOT EXISTS idx_project_tags_tag ON project_tags(tag);

CREATE INDEX IF NOT EXISTS idx_sessions_type ON sessions(session_type_id);
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);

CREATE INDEX IF NOT EXISTS idx_relationships_type ON relationships(rel_type_id);

CREATE INDEX IF NOT EXISTS idx_applicability_scope ON applicability_types(scope);
CREATE INDEX IF NOT EXISTS idx_projects_active ON projects(active);

-- Step 10: Update schema version
PRAGMA user_version = 3;

COMMIT;
