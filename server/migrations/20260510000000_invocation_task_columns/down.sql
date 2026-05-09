DROP INDEX IF EXISTS idx_tasks_remote_id;
DROP INDEX IF EXISTS idx_tasks_running;
DROP INDEX IF EXISTS idx_tasks_parent_id;

-- Sqlite supports DROP COLUMN since 3.35 (2021).
ALTER TABLE tasks DROP COLUMN spec;
ALTER TABLE tasks DROP COLUMN ended_at;
ALTER TABLE tasks DROP COLUMN remote_task_id;
ALTER TABLE tasks DROP COLUMN runner_kind;
ALTER TABLE tasks DROP COLUMN executor;
