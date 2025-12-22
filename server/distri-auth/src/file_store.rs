use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use distri_types::auth::{AuthError, AuthSecret, AuthSession, OAuth2State, ToolAuthStore};

/// File-based persistent implementation of AuthStore
/// Stores authentication sessions in JSON files in the user's home directory
#[derive(Clone)]
pub struct FileToolAuthStore {
    /// Combined auth data file path (e.g., ~/.distri/auth_sessions.json)
    auth_file: PathBuf,
    /// In-memory cache for faster access (synced with files)
    sessions_cache: std::sync::Arc<RwLock<HashMap<String, AuthSession>>>,
    secrets_cache: std::sync::Arc<RwLock<HashMap<String, AuthSecret>>>,
    oauth2_states_cache: std::sync::Arc<RwLock<HashMap<String, OAuth2State>>>,
}

/// Serializable format for storing auth sessions in JSON
#[derive(Debug, Serialize, Deserialize)]
struct StoredAuthData {
    sessions: HashMap<String, AuthSession>,
    secrets: HashMap<String, AuthSecret>,
    oauth2_states: HashMap<String, OAuth2State>,
    version: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl FileToolAuthStore {
    /// Create a new file-based auth store
    /// Sessions will be stored in ~/.distri/auth_sessions.json
    pub fn new() -> Result<Self, AuthError> {
        let home_dir = dirs::home_dir()
            .ok_or_else(|| AuthError::Storage(anyhow::anyhow!("Could not find home directory")))?;

        let distri_dir = home_dir.join(".distri");

        // Create .distri directory if it doesn't exist
        if !distri_dir.exists() {
            fs::create_dir_all(&distri_dir).map_err(|e| {
                AuthError::Storage(anyhow::anyhow!("Failed to create .distri directory: {}", e))
            })?;
        }

        let sessions_file = distri_dir.join("auth_sessions.json");

        let store = Self {
            auth_file: sessions_file,
            sessions_cache: std::sync::Arc::new(RwLock::new(HashMap::new())),
            secrets_cache: std::sync::Arc::new(RwLock::new(HashMap::new())),
            oauth2_states_cache: std::sync::Arc::new(RwLock::new(HashMap::new())),
        };

        Ok(store)
    }

    /// Initialize and load existing data from files
    pub async fn new_with_data() -> Result<Self, AuthError> {
        let store = Self::new()?;
        store.load_from_files().await?;
        Ok(store)
    }

    /// Load all data from files into cache
    async fn load_from_files(&self) -> Result<(), AuthError> {
        // Load sessions
        if self.auth_file.exists() {
            match fs::read_to_string(&self.auth_file) {
                Ok(content) => {
                    match serde_json::from_str::<StoredAuthData>(&content) {
                        Ok(stored_data) => {
                            let mut sessions = self.sessions_cache.write().await;
                            *sessions = stored_data.sessions;

                            let mut secrets = self.secrets_cache.write().await;
                            *secrets = stored_data.secrets;

                            let mut states = self.oauth2_states_cache.write().await;
                            *states = stored_data.oauth2_states;

                            debug!(
                                "Loaded {} sessions, {} secrets, {} states from files",
                                sessions.len(),
                                secrets.len(),
                                states.len()
                            );
                        }
                        Err(e) => {
                            warn!("Failed to parse auth sessions file ({}), starting fresh", e);
                            // If we can't parse the file, delete it to start fresh
                            let _ = fs::remove_file(&self.auth_file);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read auth sessions file: {}, starting fresh", e);
                }
            }
        } else {
            debug!("Auth sessions file does not exist, starting fresh");
        }

        Ok(())
    }

    /// Save all cached data to files
    async fn save_to_files(&self) -> Result<(), AuthError> {
        let sessions = self.sessions_cache.read().await;
        let secrets = self.secrets_cache.read().await;
        let states = self.oauth2_states_cache.read().await;

        let stored_data = StoredAuthData {
            sessions: sessions.clone(),
            secrets: secrets.clone(),
            oauth2_states: states.clone(),
            version: "1.0".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let json_content = serde_json::to_string_pretty(&stored_data).map_err(|e| {
            AuthError::Storage(anyhow::anyhow!("Failed to serialize auth data: {}", e))
        })?;

        fs::write(&self.auth_file, json_content).map_err(|e| {
            AuthError::Storage(anyhow::anyhow!("Failed to write auth sessions file: {}", e))
        })?;

        debug!(
            "Saved {} sessions, {} secrets, {} states to {}",
            sessions.len(),
            secrets.len(),
            states.len(),
            self.auth_file.display()
        );

        Ok(())
    }

    /// Create session key from provider and user_id
    fn make_session_key(&self, provider: &str, user_id: &str) -> String {
        format!("{}:{}", provider, user_id)
    }
}

#[async_trait]
impl ToolAuthStore for FileToolAuthStore {
    async fn store_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        session: AuthSession,
    ) -> Result<(), AuthError> {
        let session_key = self.make_session_key(auth_entity, user_id);
        debug!("Storing auth session for key: {}", session_key);

        {
            let mut sessions = self.sessions_cache.write().await;
            sessions.insert(session_key, session);
        }

        self.save_to_files().await?;
        Ok(())
    }

    async fn get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError> {
        let session_key = self.make_session_key(auth_entity, user_id);
        debug!("Getting auth session for key: {}", session_key);

        let sessions = self.sessions_cache.read().await;
        if let Some(session) = sessions.get(&session_key) {
            Ok(Some(session.clone()))
        } else {
            debug!("No session found for key: {}", session_key);
            Ok(None)
        }
    }

    async fn remove_session(&self, auth_entity: &str, user_id: &str) -> Result<bool, AuthError> {
        let session_key = self.make_session_key(auth_entity, user_id);
        debug!("Removing auth session for key: {}", session_key);

        let removed = {
            let mut sessions = self.sessions_cache.write().await;
            sessions.remove(&session_key).is_some()
        };

        if removed {
            self.save_to_files().await?;
        }
        Ok(removed)
    }

    async fn store_secret(
        &self,
        _user_id: &str, // File store assumes single user
        auth_entity: Option<&str>,
        secret: AuthSecret,
    ) -> Result<(), AuthError> {
        // For file store, we'll use a compound key: "auth_entity|key" or just "key" for global
        let storage_key = match auth_entity {
            Some(entity) => format!("{}|{}", entity, secret.key),
            None => secret.key.clone(),
        };

        debug!("Storing secret with storage key: {}", storage_key);

        {
            let mut secrets = self.secrets_cache.write().await;
            secrets.insert(storage_key, secret);
        }

        self.save_to_files().await?;
        Ok(())
    }

    async fn get_secret(
        &self,
        _user_id: &str, // File store assumes single user
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<Option<AuthSecret>, AuthError> {
        let storage_key = match auth_entity {
            Some(entity) => format!("{}|{}", entity, key),
            None => key.to_string(),
        };

        debug!("Getting secret for storage key: {}", storage_key);

        let secrets = self.secrets_cache.read().await;
        Ok(secrets.get(&storage_key).cloned())
    }

    async fn remove_secret(
        &self,
        _user_id: &str, // File store assumes single user
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<bool, AuthError> {
        let storage_key = match auth_entity {
            Some(entity) => format!("{}|{}", entity, key),
            None => key.to_string(),
        };

        debug!("Removing secret for storage key: {}", storage_key);

        let removed = {
            let mut secrets = self.secrets_cache.write().await;
            secrets.remove(&storage_key).is_some()
        };

        if removed {
            self.save_to_files().await?;
        }
        Ok(removed)
    }

    async fn store_oauth2_state(&self, state: OAuth2State) -> Result<(), AuthError> {
        debug!("Storing OAuth2 state: {}", state.state);

        {
            let mut states = self.oauth2_states_cache.write().await;
            states.insert(state.state.clone(), state);
        }

        self.save_to_files().await?;
        Ok(())
    }

    async fn get_oauth2_state(&self, state: &str) -> Result<Option<OAuth2State>, AuthError> {
        debug!("Getting OAuth2 state: {}", state);

        let states = self.oauth2_states_cache.read().await;
        if let Some(oauth_state) = states.get(state) {
            // Check if state is expired (10 minutes default)
            if oauth_state.is_expired(600) {
                debug!("OAuth2 state expired: {}", state);
                return Ok(None);
            }
            Ok(Some(oauth_state.clone()))
        } else {
            Ok(None)
        }
    }

    async fn remove_oauth2_state(&self, state: &str) -> Result<(), AuthError> {
        debug!("Removing OAuth2 state: {}", state);

        {
            let mut states = self.oauth2_states_cache.write().await;
            states.remove(state);
        }

        self.save_to_files().await?;
        Ok(())
    }
    /// Get all stored sessions (for debugging/status)
    async fn list_sessions(
        &self,
        _user_id: &str,
    ) -> Result<HashMap<String, AuthSession>, AuthError> {
        let sessions = self.sessions_cache.read().await;
        Ok(sessions.clone())
    }

    /// Get all stored secrets (for loading into context)
    async fn list_secrets(&self, _user_id: &str) -> Result<HashMap<String, AuthSecret>, AuthError> {
        let secrets = self.secrets_cache.read().await;
        Ok(secrets.clone())
    }
}

impl FileToolAuthStore {
    /// Get the path to the auth file for external access
    pub fn auth_file_path(&self) -> &PathBuf {
        &self.auth_file
    }

    /// Clear all stored authentication data
    pub async fn clear_all(&self) -> Result<(), AuthError> {
        info!("Clearing all authentication data");

        {
            let mut sessions = self.sessions_cache.write().await;
            sessions.clear();
        }
        {
            let mut secrets = self.secrets_cache.write().await;
            secrets.clear();
        }
        {
            let mut states = self.oauth2_states_cache.write().await;
            states.clear();
        }

        self.save_to_files().await?;
        Ok(())
    }

    /// Remove a specific session  
    pub async fn remove_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<bool, AuthError> {
        let session_key = self.make_session_key(auth_entity, user_id);
        debug!("Removing session for key: {}", session_key);

        let removed = {
            let mut sessions = self.sessions_cache.write().await;
            sessions.remove(&session_key).is_some()
        };

        if removed {
            self.save_to_files().await?;
        }

        Ok(removed)
    }

    /// Get status of all stored authentication
    pub async fn get_auth_status(&self) -> HashMap<String, AuthSession> {
        let sessions = self.sessions_cache.read().await;
        sessions.clone()
    }
}
