use std::sync::Arc;

use crate::{
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    types::TransportType,
    McpSession, SessionStore, ToolDefinition,
};

pub fn get_session_store() -> Option<Arc<Box<dyn SessionStore>>> {
    dotenv::dotenv().ok();
    let session_key = std::env::var("LYNEL_SESSION").unwrap();
    // Create executor with static session store

    Some(Arc::new(
        Box::new(StaticSessionStore { session_key }) as Box<dyn SessionStore>
    ))
}

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl SessionStore for StaticSessionStore {
    async fn get_session(&self, _tool_name: &str) -> anyhow::Result<Option<McpSession>> {
        Ok(Some(McpSession {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }
}

// Comment out the simple version
pub fn get_twitter_tool() -> ToolDefinition {
    ToolDefinition {
        actions_filter: crate::types::ActionsFilter::All,
        mcp_server: "twitter".to_string(),
    }
}

pub fn get_registry() -> Arc<ServerRegistry> {
    let mut registry = ServerRegistry::new();
    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Async,
            builder: Some(Arc::new(|_, transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memory: None,
        },
    );
    Arc::new(registry)
}
