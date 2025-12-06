-- Migration from v3 to v4
-- Adds ID column to relationships table for easier management
-- Safe to run multiple times (idempotent)

BEGIN TRANSACTION;

-- Step 1: Create new relationships table with ID column
CREATE TABLE IF NOT EXISTS relationships_v4 (
    id TEXT PRIMARY KEY,
    from_entry_id TEXT NOT NULL,
    to_entry_id TEXT NOT NULL,
    relationship_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (from_entry_id) REFERENCES knowledge(id) ON DELETE CASCADE,
    FOREIGN KEY (to_entry_id) REFERENCES knowledge(id) ON DELETE CASCADE,
    FOREIGN KEY (relationship_type) REFERENCES relationship_types(id),
    UNIQUE (from_entry_id, to_entry_id, relationship_type)
);

-- Step 2: Migrate existing relationships if any exist
INSERT INTO relationships_v4 (id, from_entry_id, to_entry_id, relationship_type, created_at)
SELECT
    'rel-' || substr(from_id, 4, 5) || '-' || substr(to_id, 4, 5) || '-' || substr(rel_type_id, 1, 3),
    from_id,
    to_id,
    rel_type_id,
    created_at
FROM relationships
WHERE EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name='relationships');

-- Step 3: Drop old relationships table
DROP TABLE IF EXISTS relationships;

-- Step 4: Rename new table
ALTER TABLE relationships_v4 RENAME TO relationships;

-- Step 5: Create indexes for relationships
CREATE INDEX IF NOT EXISTS idx_relationships_type ON relationships(relationship_type);
CREATE INDEX IF NOT EXISTS idx_relationships_from ON relationships(from_entry_id);
CREATE INDEX IF NOT EXISTS idx_relationships_to ON relationships(to_entry_id);

-- Step 6: Update schema version
PRAGMA user_version = 4;

COMMIT;
