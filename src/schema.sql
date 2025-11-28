-- Knowledge entries (Zion)
CREATE TABLE IF NOT EXISTS knowledge (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,        -- pattern, technique, insight, ritual, project
    title TEXT NOT NULL,
    body TEXT,
    summary TEXT,                  -- First paragraph or explicit summary
    applicability TEXT,            -- When/where this applies
    source_project TEXT,           -- Where it was learned
    source_agent TEXT,             -- Who captured it
    file_path TEXT,                -- Original markdown path
    tags TEXT,                     -- JSON array
    created_at TEXT,
    updated_at TEXT,
    content_hash TEXT,             -- For change detection

    -- Provenance metadata
    source_type TEXT,              -- manual, ram, cache, agent_session
    entry_type TEXT,               -- primary, summary, synthesis
    session_id TEXT,               -- Link to originating session (if from RAM)
    ephemeral INTEGER DEFAULT 0    -- 1 = session-based, may be pruned
);

-- Tags index for fast tag queries
CREATE TABLE IF NOT EXISTS tags (
    entry_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (entry_id, tag),
    FOREIGN KEY (entry_id) REFERENCES knowledge(id) ON DELETE CASCADE
);

-- Relationships between entries
CREATE TABLE IF NOT EXISTS relationships (
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    rel_type TEXT NOT NULL,        -- related, supersedes, extends
    created_at TEXT,
    PRIMARY KEY (from_id, to_id, rel_type)
);

-- Deletions tombstones for sync
CREATE TABLE IF NOT EXISTS deletions (
    id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_knowledge_category ON knowledge(category);
CREATE INDEX IF NOT EXISTS idx_knowledge_updated ON knowledge(updated_at);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
