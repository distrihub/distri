-- Add channel_id to threads (was missing from SQLite migrations)
ALTER TABLE threads ADD COLUMN channel_id TEXT;

-- Add token usage tracking to threads
ALTER TABLE threads ADD COLUMN input_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN output_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN total_tokens BIGINT NOT NULL DEFAULT 0;
