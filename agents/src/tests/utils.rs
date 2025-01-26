use std::sync::Arc;

use crate::{
    types::{AuthType, TransportType},
    Session, SessionStore, ToolDefinition,
};

pub fn get_session_store() -> Option<Arc<Box<dyn SessionStore>>> {
    dotenv::dotenv().ok();
    let session_key = std::env::var("LYNEL_SESSION").unwrap();
    // Create executor with static session store
    let session_store = Some(Arc::new(
        Box::new(StaticSessionStore { session_key }) as Box<dyn SessionStore>
    ));
    session_store
}

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl SessionStore for StaticSessionStore {
    async fn save_session(&self, _tool_name: &str, _session: Session) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_session(&self, _tool_name: &str) -> anyhow::Result<Option<Session>> {
        Ok(Some(Session {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }

    async fn delete_session(&self, _tool_name: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

// Comment out the simple version
pub fn get_twitter_tool() -> ToolDefinition {
    ToolDefinition {
        actions_filter: crate::types::ActionsFilter::All,
        auth_type: AuthType::None,
        auth_session_key: Some("session_string".to_string()),
        mcp_transport: TransportType::Async,
        mcp_server: "twitter".to_string(),
    }
}

// pub fn get_twitter_tool() -> ToolDefinition {
//     ToolDefinition {
//         tools: vec![mcp_sdk::types::Tool {
//             name: "get_timeline".to_string(),
//             description: Some("Get user's home timeline".to_string()),
//             input_schema: serde_json::json!({
//                 "type": "object",
//                 "properties": {
//                     "count": {"type": "integer", "default": 5}
//                 },
//                 "required": []
//             }),
//         }],
//         auth_type: AuthType::None,
//         auth_session_key: Some("session_string".to_string()),
//         mcp_transport: TransportType::Async,
//         mcp_server: "twitter".to_string(),
//     }
// }
