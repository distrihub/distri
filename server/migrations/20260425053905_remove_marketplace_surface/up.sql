-- OSS-side: drop marketplace columns from skills.
-- Skills are workspace-scoped only; "public" lives in external Discover.
DROP INDEX IF EXISTS idx_skills_is_public;

ALTER TABLE skills DROP COLUMN is_public;
ALTER TABLE skills DROP COLUMN is_system;
ALTER TABLE skills DROP COLUMN star_count;
ALTER TABLE skills DROP COLUMN clone_count;
