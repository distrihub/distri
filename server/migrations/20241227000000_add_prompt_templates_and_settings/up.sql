-- Create prompt_templates table for storing user prompt templates
CREATE TABLE IF NOT EXISTS prompt_templates (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    template TEXT NOT NULL,
    description TEXT,
    version TEXT,
    is_system INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Create index on name for faster lookups
CREATE INDEX IF NOT EXISTS idx_prompt_templates_name ON prompt_templates(name);
CREATE INDEX IF NOT EXISTS idx_prompt_templates_is_system ON prompt_templates(is_system);

-- Create server_settings table for storing DistriServerConfig
CREATE TABLE IF NOT EXISTS server_settings (
    id TEXT PRIMARY KEY NOT NULL DEFAULT 'default',
    config_json TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Create secrets table for storing API key secrets
CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY NOT NULL,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_secrets_key ON secrets(key);