-- Your SQL goes here
CREATE TABLE agents (
  id SERIAL PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  description TEXT,
  tools JSONB,
  model VARCHAR(255) NOT NULL,
  model_settings JSONB,
  provider_name VARCHAR(255) NOT NULL,
  prompt TEXT,
  avatar TEXT,
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Add trigger for updating the updated_at timestamp
SELECT diesel_manage_updated_at('agents');