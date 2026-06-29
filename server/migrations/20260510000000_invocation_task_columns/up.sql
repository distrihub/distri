-- Sqlite parallel of pg_migrations/20260510000000_invocation_task_columns
-- for OSS distri-server. Same Invocation columns; type adjustments:
--   * BOOLEAN → INTEGER (0 / 1)
--   * JSONB   → TEXT (json_extract works in sqlite >= 3.38)
--   * TIMESTAMPTZ-equivalents → BIGINT (epoch-ms, matches existing
--                                       created_at convention)

ALTER TABLE tasks ADD COLUMN remote INTEGER NOT NULL DEFAULT 0
    CHECK (remote IN (0, 1));
ALTER TABLE tasks ADD COLUMN inner_task_id TEXT;
ALTER TABLE tasks ADD COLUMN ended_at BIGINT;
ALTER TABLE tasks ADD COLUMN invocation TEXT NOT NULL DEFAULT '{}';

-- Sqlite ALTER TABLE doesn't support adding multi-column CHECK
-- constraints after the fact. The cross-column invariant
-- (remote = false ⇒ inner_task_id IS NULL) is enforced at the
-- application layer by `Invocation::validate` plus the value stored
-- in `remote`.

CREATE INDEX IF NOT EXISTS idx_tasks_parent_id ON tasks(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_running ON tasks(thread_id, created_at)
    WHERE status = 'running';
CREATE INDEX IF NOT EXISTS idx_tasks_inner_id ON tasks(inner_task_id)
    WHERE inner_task_id IS NOT NULL;
