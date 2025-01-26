-- Create users table
CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  twitter_id VARCHAR(128) NOT NULL UNIQUE,
  name VARCHAR(255) NOT NULL,
  description TEXT,
  location VARCHAR(255),
  twitter_url VARCHAR(255),
  profile_image_url VARCHAR(255),
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Add tags and user_id to agents
ALTER TABLE agents
ADD COLUMN user_id INTEGER REFERENCES users(id);

-- Add trigger for updating users updated_at
SELECT diesel_manage_updated_at('users');