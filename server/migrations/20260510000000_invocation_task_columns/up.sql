-- Sqlite parallel of pg_migrations/20260510000000_invocation_task_columns
-- for OSS distri-server. Same Invocation columns; type adjustments:
--   * BOOLEAN → INTEGER (0 / 1)
--   * JSONB   → TEXT (json_extract works in sqlite >= 3.38)
--   * TIMESTAMPTZ-equivalents → BIGINT (epoch-ms, matches the existing
--                                       created_at convention)
-- CHECK constraints are supported by sqlite identically.

ALTER TABLE tasks ADD COLUMN executor TEXT NOT NULL DEFAULT 'local'
    CHECK (executor IN ('local', 'remote_sandbox', 'remote_loopback'));
ALTER TABLE tasks ADD COLUMN inner_task_id TEXT;
ALTER TABLE tasks ADD COLUMN ended_at BIGINT;
ALTER TABLE tasks ADD COLUMN invocation TEXT NOT NULL DEFAULT '{}';

-- Sqlite ALTER TABLE doesn't support adding CHECK constraints over
-- multiple existing columns after the fact; the cross-column invariant
-- (local ⇒ no inner_task_id) is enforced at the application layer by
-- `Invocation::validate` plus the value stored in `executor`.

CREATE INDEX IF NOT EXISTS idx_tasks_parent_id ON tasks(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_running ON tasks(thread_id, created_at)
    WHERE status = 'running';
CREATE INDEX IF NOT EXISTS idx_tasks_inner_id ON tasks(inner_task_id)
    WHERE inner_task_id IS NOT NULL;
