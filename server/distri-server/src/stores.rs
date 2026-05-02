//! In-process store implementations for distri-server.
//!
//! These are used when no persistent store backend is configured (e.g. in
//! tests or single-tenant local runs that haven't yet been wired to SQLite).
//! They are **not** recommended for production; Task 5 adds the SQLite-backed
//! implementations to distri-stores.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::connections::{
    AuthType, Connection, ConnectionStatus, ConnectionToken, NewConnection,
};
use distri_types::stores::{ConnectionStore, ConnectionTokenStore};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── In-memory ConnectionStore ──────────────────────────────────────────────

pub struct InMemoryConnectionStore {
    connections: RwLock<HashMap<Uuid, Connection>>,
}

impl InMemoryConnectionStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connections: RwLock::new(HashMap::new()),
        })
    }
}

impl Default for InMemoryConnectionStore {
    fn default() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ConnectionStore for InMemoryConnectionStore {
    async fn create(&self, new_conn: NewConnection) -> anyhow::Result<Connection> {
        let now = chrono::Utc::now();
        let conn = Connection {
            id: Uuid::new_v4(),
            workspace_id: new_conn.workspace_id,
            skill_id: new_conn.skill_id,
            name: new_conn.name,
            status: new_conn.status,
            config: new_conn.config,
            connected_by: new_conn.connected_by,
            created_at: now,
            updated_at: now,
            auth_scope: new_conn.auth_scope,
            auth_type: new_conn.auth_type,
            is_system: new_conn.is_system,
        };
        self.connections.write().await.insert(conn.id, conn.clone());
        Ok(conn)
    }

    async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Connection>> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        Ok(self.connections.read().await.get(&id).cloned())
    }

    async fn list_by_workspace(&self, _workspace_id: &str) -> anyhow::Result<Vec<Connection>> {
        // In single-tenant mode all connections belong to the one workspace.
        Ok(self.connections.read().await.values().cloned().collect())
    }

    async fn update_status(&self, id: &str, status: ConnectionStatus) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        if let Some(conn) = map.get_mut(&id) {
            conn.status = status;
            conn.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn update_skill_id(&self, id: &str, skill_id: Uuid) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        if let Some(conn) = map.get_mut(&id) {
            conn.skill_id = skill_id;
            conn.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        auth_type: Option<AuthType>,
    ) -> anyhow::Result<Connection> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        let conn = map
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("connection not found"))?;
        if let Some(n) = name {
            conn.name = n;
        }
        if let Some(at) = auth_type {
            conn.auth_type = at;
        }
        conn.updated_at = chrono::Utc::now();
        Ok(conn.clone())
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        self.connections.write().await.remove(&id);
        Ok(())
    }

    async fn get_by_provider(
        &self,
        _workspace_id: &str,
        provider: &str,
    ) -> anyhow::Result<Option<Connection>> {
        let map = self.connections.read().await;
        let found = map.values().find(|c| match &c.auth_type {
            AuthType::OAuth { provider: p, .. } => p == provider,
            _ => false,
        });
        Ok(found.cloned())
    }
}

// ── In-memory ConnectionTokenStore ───────────────────────────────────────────

pub struct InMemoryConnectionTokenStore {
    tokens: RwLock<HashMap<String, ConnectionToken>>,
    oauth_states: RwLock<HashMap<String, serde_json::Value>>,
}

impl InMemoryConnectionTokenStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tokens: RwLock::new(HashMap::new()),
            oauth_states: RwLock::new(HashMap::new()),
        })
    }
}

impl Default for InMemoryConnectionTokenStore {
    fn default() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            oauth_states: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ConnectionTokenStore for InMemoryConnectionTokenStore {
    async fn store_token(&self, connection_id: &str, token: ConnectionToken) -> anyhow::Result<()> {
        self.tokens
            .write()
            .await
            .insert(connection_id.to_string(), token);
        Ok(())
    }

    async fn get_token(&self, connection_id: &str) -> anyhow::Result<Option<ConnectionToken>> {
        Ok(self.tokens.read().await.get(connection_id).cloned())
    }

    async fn remove_token(&self, connection_id: &str) -> anyhow::Result<()> {
        self.tokens.write().await.remove(connection_id);
        Ok(())
    }

    async fn store_oauth_state(
        &self,
        state_key: &str,
        state: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.oauth_states
            .write()
            .await
            .insert(state_key.to_string(), state);
        Ok(())
    }

    async fn get_oauth_state(&self, state_key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(self.oauth_states.read().await.get(state_key).cloned())
    }

    async fn remove_oauth_state(&self, state_key: &str) -> anyhow::Result<()> {
        self.oauth_states.write().await.remove(state_key);
        Ok(())
    }
}
