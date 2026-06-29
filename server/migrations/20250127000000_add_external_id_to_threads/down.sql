-- Remove external_id column and indexes
DROP INDEX IF EXISTS idx_threads_external_id;
DROP INDEX IF EXISTS idx_threads_agent_id;
ALTER TABLE threads DROP COLUMN external_id;
