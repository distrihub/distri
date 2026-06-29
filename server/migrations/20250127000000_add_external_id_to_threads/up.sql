-- Add external_id column to threads table
ALTER TABLE threads ADD COLUMN external_id TEXT;

-- Create indexes for efficient filtering
CREATE INDEX IF NOT EXISTS idx_threads_agent_id ON threads(agent_id);
CREATE INDEX IF NOT EXISTS idx_threads_external_id ON threads(external_id) WHERE external_id IS NOT NULL;
