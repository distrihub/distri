PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS plugin_catalog (
  package_name TEXT PRIMARY KEY,
  version TEXT,
  object_prefix TEXT NOT NULL,
  entrypoint TEXT,
  artifact_json TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE INDEX IF NOT EXISTS idx_plugin_catalog_updated_at ON plugin_catalog(updated_at);