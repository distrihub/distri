use async_trait::async_trait;

use crate::Session;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save_session(&self, tool_name: &str, session: Session) -> anyhow::Result<()>;
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<Session>>;
    async fn delete_session(&self, tool_name: &str) -> anyhow::Result<()>;
}

// Example in-memory implementation
pub struct InMemorySessionStore {
    sessions: tokio::sync::RwLock<std::collections::HashMap<String, Session>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn save_session(&self, tool_name: &str, session: Session) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(tool_name.to_string(), session);
        Ok(())
    }

    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<Session>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(tool_name).cloned())
    }

    async fn delete_session(&self, tool_name: &str) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(tool_name);
        Ok(())
    }
}
