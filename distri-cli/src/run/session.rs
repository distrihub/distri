use std::{collections::HashMap, sync::Arc};

use distri::{agent::ExecutorContext, McpSession, ToolSessionStore};

pub fn get_session_store(sessions: HashMap<String, String>) -> Arc<Box<dyn ToolSessionStore>> {
    Arc::new(Box::new(ConfigSessionStore { sessions }))
}

pub struct ConfigSessionStore {
    sessions: HashMap<String, String>,
}

#[async_trait::async_trait]
impl ToolSessionStore for ConfigSessionStore {
    async fn get_session(
        &self,
        tool_name: &str,
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(self.sessions.get(tool_name).map(|s| McpSession {
            token: s.clone(),
            expiry: None,
        }))
    }
}
