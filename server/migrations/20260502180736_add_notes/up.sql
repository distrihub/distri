-- Notes: user-created text notes persisted per workspace.
-- In single-tenant OSS mode workspace_id is always the nil UUID.
CREATE TABLE IF NOT EXISTS notes (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    title        TEXT NOT NULL,
    content      TEXT NOT NULL,
    tags         TEXT NOT NULL DEFAULT '[]',  -- JSON array
    created_by   TEXT,                         -- nullable UUID string
    created_at   TIMESTAMP NOT NULL,
    updated_at   TIMESTAMP NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notes_workspace   ON notes(workspace_id);
CREATE INDEX IF NOT EXISTS idx_notes_updated_at  ON notes(updated_at DESC);
