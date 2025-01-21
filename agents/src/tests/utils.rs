use std::sync::Arc;

use crate::{
    types::{AuthType, TransportType},
    Session, SessionStore, ToolDefinition,
};

pub fn get_session_store() -> Option<Arc<Box<dyn SessionStore>>> {
    let session_key  = "guest_id_ads=v1%3A173719111551730639; kdt=T60Y7Gq1uM5JVTHqobkNQuxyAU2BOKlO8b3Gjzew; att=1-hS68dcUEf9FBYnfPFJKyG8UD1EWI0lHjsAYkU3xp; auth_token=c9d46f0b963dcaf2a2477e5b762c1abdcddabd95; personalization_id=v1_aUq4PsJLBR1VW/Rvsyi4ig==; guest_id_marketing=v1%3A173719111551730639; guest_id=v1%3A173719111551730639; twid=u=1497801936669913089; ct0=08ca694202f67ea16ac905516c64bf91838c6fe9e3f5680e66f1eac6c9d99f81aea56b1bd77964325d63a97dd86bce122b47d779d36221de420ea869fdd5f50fc5b33105373be8e45b695f991e01b3bb".to_string();
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

pub fn get_twitter_tool() -> ToolDefinition {
    ToolDefinition {
        tool: mcp_sdk::types::Tool {
            name: "get_timeline".to_string(),
            description: Some("Get user's home timeline".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "count": {"type": "integer", "default": 5}
                },
                "required": []
            }),
        },
        auth_type: AuthType::None,
        auth_session_key: Some("session_string".to_string()),
        mcp_transport: TransportType::Async,
        mcp_server: "twitter".to_string(),
    }
}
