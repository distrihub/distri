CREATE TABLE user_memory (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL,
    memory TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    valid_until TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id)
);
CREATE INDEX idx_user_memory_user_id_valid_until ON user_memory(user_id, valid_until);