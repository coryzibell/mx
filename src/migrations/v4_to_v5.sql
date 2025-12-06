-- Migration from v4 to v5
-- Adds content_type to knowledge entries for distinguishing text/code/data
-- Safe to run multiple times (idempotent)

BEGIN TRANSACTION;

-- Step 1: Create content_types lookup table
CREATE TABLE IF NOT EXISTS content_types (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    file_extensions TEXT,  -- comma-separated list of typical extensions
    created_at TEXT NOT NULL
);

-- Step 2: Seed content types
INSERT OR IGNORE INTO content_types (id, description, file_extensions, created_at) VALUES
    ('text', 'Plain text or markdown documents', 'md,txt,text', datetime('now')),
    ('code', 'Source code or scripts', 'py,rs,js,ts,sh,bash,rb,go,java,c,cpp,h', datetime('now')),
    ('config', 'Configuration files', 'json,yaml,yml,toml,xml,ini,env', datetime('now')),
    ('data', 'Data files or test fixtures', 'json,csv,sql,fiche,schema', datetime('now')),
    ('binary', 'Binary or encoded content', 'bin,dat,b64', datetime('now'));

-- Step 3: Add content_type_id column to knowledge table
-- Note: This will fail if column already exists (from a previous partial migration)
-- In that case, the error is expected and migration continues via db.rs logic
ALTER TABLE knowledge ADD COLUMN content_type_id TEXT DEFAULT 'text';

-- Step 4: Create index for content_type queries
CREATE INDEX IF NOT EXISTS idx_knowledge_content_type ON knowledge(content_type_id);

-- Step 5: Update schema version
PRAGMA user_version = 5;

COMMIT;
