-- Create users table
CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  firebase_id VARCHAR(128) NOT NULL UNIQUE,
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Add tags and user_id to agents
ALTER TABLE agents
ADD COLUMN user_id INTEGER REFERENCES users(id),
  ADD COLUMN tags TEXT [];

-- Add trigger for updating users updated_at
SELECT diesel_manage_updated_at('users');