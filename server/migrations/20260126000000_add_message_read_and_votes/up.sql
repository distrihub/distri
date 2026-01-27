-- Message read status table
-- Tracks when users have read messages in a thread
CREATE TABLE IF NOT EXISTS message_reads (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    read_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Each user can only have one read record per message
    UNIQUE (thread_id, message_id, user_id)
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_message_reads_thread_id ON message_reads(thread_id);
CREATE INDEX IF NOT EXISTS idx_message_reads_user_id ON message_reads(user_id);
CREATE INDEX IF NOT EXISTS idx_message_reads_message_id ON message_reads(message_id);

-- Message votes table
-- Tracks upvotes and downvotes on messages with optional comments
CREATE TABLE IF NOT EXISTS message_votes (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    vote_type TEXT NOT NULL CHECK (vote_type IN ('upvote', 'downvote')),
    comment TEXT, -- Required for downvotes, optional for upvotes
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Each user can only vote once per message
    UNIQUE (thread_id, message_id, user_id)
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_message_votes_thread_id ON message_votes(thread_id);
CREATE INDEX IF NOT EXISTS idx_message_votes_user_id ON message_votes(user_id);
CREATE INDEX IF NOT EXISTS idx_message_votes_message_id ON message_votes(message_id);
CREATE INDEX IF NOT EXISTS idx_message_votes_vote_type ON message_votes(vote_type);
