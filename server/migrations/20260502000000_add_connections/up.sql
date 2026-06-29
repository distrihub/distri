-- Connections: persists OAuth/custom API connections for a workspace.
-- In single-tenant OSS mode workspace_id is always the nil UUID.
CREATE TABLE IF NOT EXISTS connections (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    skill_id     TEXT NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    name         TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    config       TEXT NOT NULL DEFAULT '{}',
    connected_by TEXT,
    auth_scope   TEXT NOT NULL DEFAULT 'Workspace',
    auth_type    TEXT NOT NULL DEFAULT '{"Custom":{"fields":[]}}',
    is_system    INTEGER NOT NULL DEFAULT 0,
    created_at   TIMESTAMP NOT NULL,
    updated_at   TIMESTAMP NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_connections_workspace ON connections(workspace_id);
CREATE INDEX IF NOT EXISTS idx_connections_skill ON connections(skill_id);

-- Connection tokens: stores OAuth access/refresh tokens keyed by connection_id.
CREATE TABLE IF NOT EXISTS connection_tokens (
    connection_id TEXT PRIMARY KEY NOT NULL,
    token_json    TEXT NOT NULL,
    created_at    TIMESTAMP NOT NULL,
    updated_at    TIMESTAMP NOT NULL
);

-- OAuth state for in-flight OAuth flows: keyed by state string.
CREATE TABLE IF NOT EXISTS connection_oauth_states (
    state_key  TEXT PRIMARY KEY NOT NULL,
    state_json TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL
);
