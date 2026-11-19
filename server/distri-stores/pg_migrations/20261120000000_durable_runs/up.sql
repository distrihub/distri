-- Durable runs (postgres) for the distri-stores Diesel schema (tasks.created_at
-- is epoch-ms BIGINT there, JSON is TEXT). Same columns as the sqlite
-- migration in server/migrations/20261120000000_durable_runs.
-- A host with its own tasks table (distri-cloud) uses timestamptz/jsonb and
-- its own RunQueue/PendingCallStore; this file is for the OSS pg backend.

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS ready_at BIGINT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS lease_owner TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS lease_until BIGINT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS lease_id TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS waiting_on TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS run_inputs TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS run_state TEXT;

CREATE INDEX IF NOT EXISTS idx_tasks_ready_at ON tasks(ready_at)
    WHERE ready_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_lease_until ON tasks(lease_until)
    WHERE lease_until IS NOT NULL;

CREATE TABLE IF NOT EXISTS pending_calls (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT,
    task_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    tool_name TEXT,
    request TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    completed_by TEXT,
    deadline BIGINT,
    created_at BIGINT NOT NULL,
    completed_at BIGINT,
    claimed_by TEXT,
    claimed_until BIGINT
);
CREATE INDEX IF NOT EXISTS idx_pending_calls_task ON pending_calls(task_id, status);
CREATE INDEX IF NOT EXISTS idx_pending_calls_thread ON pending_calls(thread_id, status);
CREATE INDEX IF NOT EXISTS idx_pending_calls_deadline ON pending_calls(deadline)
    WHERE status = 'pending';
