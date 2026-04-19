-- Last-known ContextBudget snapshot per thread. Stored as TEXT (JSON) on
-- SQLite, JSONB on Postgres (column type coerces via the shared Jsonb alias
-- in distri-stores/src/schema.rs).

ALTER TABLE threads ADD COLUMN last_context_budget TEXT;
