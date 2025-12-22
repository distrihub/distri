use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use distri_types::{
    ToolAuthStore,
    auth::{AuthError, AuthSecret, AuthSession, OAuth2State},
};

/// In-memory implementation of AuthStore for development and testing
#[derive(Clone)]
pub struct InMemoryToolAuthStore {
    /// Stored authentication sessions by (user_id, auth_entity)
    sessions: Arc<RwLock<HashMap<(String, String), AuthSession>>>,
    /// Stored secrets by user_id -> auth_entity -> secret_key -> AuthSecret (None auth_entity = global)
    secrets: Arc<RwLock<HashMap<String, HashMap<Option<String>, HashMap<String, AuthSecret>>>>>,
    /// OAuth2 states by state parameter
    oauth2_states: Arc<RwLock<HashMap<String, OAuth2State>>>,
}

impl InMemoryToolAuthStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            secrets: Arc::new(RwLock::new(HashMap::new())),
            oauth2_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn make_key(&self, user_id: &str, auth_entity: &str) -> (String, String) {
        let user_key = user_id.to_string();
        (user_key, auth_entity.to_string())
    }
}

#[async_trait]
impl ToolAuthStore for InMemoryToolAuthStore {
    async fn get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError> {
        debug!(
            "Getting session for entity: {} user: {:?}",
            auth_entity, user_id
        );

        let key = self.make_key(user_id, auth_entity);
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&key).cloned())
    }

    async fn store_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        session: AuthSession,
    ) -> Result<(), AuthError> {
        debug!(
            "Storing session for entity: {} user: {:?}",
            auth_entity, user_id
        );

        let key = self.make_key(user_id, auth_entity);
        let mut sessions = self.sessions.write().await;
        sessions.insert(key, session);

        Ok(())
    }

    async fn remove_session(&self, auth_entity: &str, user_id: &str) -> Result<bool, AuthError> {
        debug!(
            "Removing session for entity: {} user: {:?}",
            auth_entity, user_id
        );

        let key = self.make_key(user_id, auth_entity);
        let mut sessions = self.sessions.write().await;
        Ok(sessions.remove(&key).is_some())
    }

    async fn store_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        secret: AuthSecret,
    ) -> Result<(), AuthError> {
        debug!(
            "Storing secret for user: {:?}, auth_entity: {:?}, key: {}",
            user_id, auth_entity, secret.key
        );

        let user_key = user_id.to_string();
        let auth_entity_key = auth_entity.map(|s| s.to_string());

        let mut secrets = self.secrets.write().await;
        let user_secrets = secrets.entry(user_key).or_insert_with(HashMap::new);
        let entity_secrets = user_secrets
            .entry(auth_entity_key)
            .or_insert_with(HashMap::new);
        entity_secrets.insert(secret.key.clone(), secret);
        Ok(())
    }

    async fn get_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<Option<AuthSecret>, AuthError> {
        debug!(
            "Getting secret for user: {:?}, auth_entity: {:?}, key: {}",
            user_id, auth_entity, key
        );

        let user_key = user_id.to_string();
        let auth_entity_key = auth_entity.map(|s| s.to_string());

        let secrets = self.secrets.read().await;
        Ok(secrets
            .get(&user_key)
            .and_then(|user_secrets| user_secrets.get(&auth_entity_key))
            .and_then(|entity_secrets| entity_secrets.get(key).cloned()))
    }

    async fn remove_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<bool, AuthError> {
        debug!(
            "Removing secret for user: {:?}, auth_entity: {:?}, key: {}",
            user_id, auth_entity, key
        );

        let user_key = user_id.to_string();
        let auth_entity_key = auth_entity.map(|s| s.to_string());

        let mut secrets = self.secrets.write().await;
        Ok(secrets
            .get_mut(&user_key)
            .and_then(|user_secrets| user_secrets.get_mut(&auth_entity_key))
            .map(|entity_secrets| entity_secrets.remove(key).is_some())
            .unwrap_or(false))
    }

    async fn store_oauth2_state(&self, state: OAuth2State) -> Result<(), AuthError> {
        debug!("Storing OAuth2 state: {}", state.state);

        let mut states = self.oauth2_states.write().await;
        states.insert(state.state.clone(), state);
        Ok(())
    }

    async fn get_oauth2_state(&self, state: &str) -> Result<Option<OAuth2State>, AuthError> {
        debug!("Getting OAuth2 state: {}", state);

        let states = self.oauth2_states.read().await;
        Ok(states.get(state).cloned())
    }

    async fn remove_oauth2_state(&self, state: &str) -> Result<(), AuthError> {
        debug!("Removing OAuth2 state: {}", state);

        let mut states = self.oauth2_states.write().await;
        states.remove(state);
        Ok(())
    }

    async fn list_secrets(&self, user_id: &str) -> Result<HashMap<String, AuthSecret>, AuthError> {
        let user_key = user_id.to_string();
        let secrets = self.secrets.read().await;

        let mut result = HashMap::new();
        if let Some(user_secrets) = secrets.get(&user_key) {
            for (_auth_entity, entity_secrets) in user_secrets {
                for (key, secret) in entity_secrets {
                    // Flatten all secrets regardless of auth_entity scope
                    result.insert(key.clone(), secret.clone());
                }
            }
        }
        Ok(result)
    }

    async fn list_sessions(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, AuthSession>, AuthError> {
        let user_key = user_id.to_string();
        let sessions = self.sessions.read().await;

        let result = sessions
            .iter()
            .filter_map(|((uid, auth_entity), session)| {
                if uid == &user_key {
                    Some((auth_entity.clone(), session.clone()))
                } else {
                    None
                }
            })
            .collect();
        Ok(result)
    }
}
