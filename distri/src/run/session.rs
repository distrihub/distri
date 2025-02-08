use std::{collections::HashMap, sync::Arc};

use distri::{McpSession, ToolSessionStore};

pub fn get_session_store(
    sessions: HashMap<String, String>,
) -> Option<Arc<Box<dyn ToolSessionStore>>> {
    Some(Arc::new(
        Box::new(ConfigSessionStore { sessions }) as Box<dyn ToolSessionStore>
    ))
}

pub struct ConfigSessionStore {
    sessions: HashMap<String, String>,
}

#[async_trait::async_trait]
impl ToolSessionStore for ConfigSessionStore {
    async fn get_session(&self, tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        Ok(self.sessions.get(tool_name).map(|s| McpSession {
            token: s.clone(),
            expiry: None,
        }))
    }
}
