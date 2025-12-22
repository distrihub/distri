-- Core schema for SQLite (used by distri-stores). Uses IF NOT EXISTS to allow idempotent setup.

-- Threads
CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY NOT NULL,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    message_count INTEGER NOT NULL DEFAULT 0,
    last_message TEXT,
    metadata TEXT NOT NULL,
    attributes TEXT NOT NULL
);

-- Tasks
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    thread_id TEXT NOT NULL,
    parent_task_id TEXT,
    status TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_thread ON tasks(thread_id);

-- Task messages/events
CREATE TABLE IF NOT EXISTS task_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_messages_task ON task_messages(task_id);

-- Session entries
CREATE TABLE IF NOT EXISTS session_entries (
    thread_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    expiry TIMESTAMP,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    PRIMARY KEY(thread_id, key)
);

-- Memory entries
CREATE TABLE IF NOT EXISTS memory_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_entries_user ON memory_entries(user_id);

-- Scratchpad entries
CREATE TABLE IF NOT EXISTS scratchpad_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    parent_task_id TEXT,
    entry TEXT NOT NULL,
    entry_type TEXT,
    timestamp BIGINT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    FOREIGN KEY(parent_task_id) REFERENCES tasks(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_scratchpad_thread ON scratchpad_entries(thread_id);
CREATE INDEX IF NOT EXISTS idx_scratchpad_task ON scratchpad_entries(task_id);
CREATE INDEX IF NOT EXISTS idx_scratchpad_parent_task ON scratchpad_entries(parent_task_id);

-- Agent configs
CREATE TABLE IF NOT EXISTS agent_configs (
    name TEXT PRIMARY KEY NOT NULL,
    config TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- Integrations
CREATE TABLE IF NOT EXISTS integrations (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    session_data TEXT,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    expires_at TIMESTAMP
);

-- Plugin catalog (ensure proper schema by recreating)
DROP TABLE IF EXISTS plugin_catalog;
CREATE TABLE plugin_catalog (
    package_name TEXT PRIMARY KEY NOT NULL,
    version TEXT,
    object_prefix TEXT NOT NULL,
    entrypoint TEXT,
    artifact_json TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- External tool calls
CREATE TABLE IF NOT EXISTS external_tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    request TEXT NOT NULL,
    response TEXT,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);
