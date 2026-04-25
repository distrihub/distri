-- Restore marketplace columns. Data is unrecoverable.
ALTER TABLE skills ADD COLUMN is_public INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN star_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN clone_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_skills_is_public ON skills(is_public);
