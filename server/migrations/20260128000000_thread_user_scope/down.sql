-- Remove user-scoped thread uniqueness
DROP INDEX IF EXISTS idx_threads_id_user;
ALTER TABLE threads DROP COLUMN user_id;
