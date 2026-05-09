DROP INDEX IF EXISTS idx_tasks_inner_id;
DROP INDEX IF EXISTS idx_tasks_running;
DROP INDEX IF EXISTS idx_tasks_parent_id;

-- Sqlite supports DROP COLUMN since 3.35 (2021).
ALTER TABLE tasks DROP COLUMN invocation;
ALTER TABLE tasks DROP COLUMN ended_at;
ALTER TABLE tasks DROP COLUMN inner_task_id;
ALTER TABLE tasks DROP COLUMN executor;
