-- Add user_id to threads for user-scoped thread uniqueness
ALTER TABLE threads ADD COLUMN user_id TEXT NOT NULL DEFAULT '';

-- Create unique index on (id, user_id) so thread IDs are user-scoped
CREATE UNIQUE INDEX IF NOT EXISTS idx_threads_id_user ON threads(id, user_id);
