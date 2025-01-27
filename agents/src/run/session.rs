use std::{collections::HashMap, sync::Arc};

use agents::{McpSession, SessionStore};

pub fn get_session_store(sessions: HashMap<String, String>) -> Option<Arc<Box<dyn SessionStore>>> {
    let session_store = Some(Arc::new(
        Box::new(ConfigSessionStore { sessions }) as Box<dyn SessionStore>
    ));
    session_store
}

pub struct ConfigSessionStore {
    sessions: HashMap<String, String>,
}

#[async_trait::async_trait]
impl SessionStore for ConfigSessionStore {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        Ok(self.sessions.get(tool_name).map(|s| McpSession {
            token: s.clone(),
            expiry: None,
        }))
    }
}
