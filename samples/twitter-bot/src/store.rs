use std::sync::Arc;

use distri::{agent::ExecutorContext, types::McpSession, ToolSessionStore};

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl ToolSessionStore for StaticSessionStore {
    async fn get_session(
        &self,
        _tool_name: &str,
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(Some(McpSession {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }
}

pub fn get_tools_session_store() -> Arc<Box<dyn ToolSessionStore>> {
    dotenv::dotenv().ok();
    let session_key =
        std::env::var("X_USER_SESSION").unwrap_or_else(|_| "test_session_key".to_string());
    // Create executor with static session store

    Arc::new(Box::new(StaticSessionStore { session_key }))
}
