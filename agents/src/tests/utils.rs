use std::sync::Arc;

use crate::{
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    types::TransportType,
    AgentDefinition, McpDefinition, McpSession, ModelSettings, SessionStore,
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
pub fn get_twitter_tool() -> McpDefinition {
    McpDefinition {
        filter: crate::types::ToolsFilter::All,
        mcp_server: "twitter".to_string(),
        mcp_server_type: Default::default(),
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

pub static SYSTEM_PROMPT: &str = r#"You are a helpful AI assistant that can access Twitter and summarize information.
When asked about tweets, you will:
1. Get the timeline using the Twitter tool
2. Format the tweets in a clean markdown format
3. Add brief summaries and insights
4. Group similar tweets together by theme
5. Highlight particularly interesting or important tweets
6. You dont need to login; Session is already available. 

Keep your summaries concise but informative. Use markdown formatting to make the output readable."#;

pub fn get_twitter_summarizer() -> AgentDefinition {
    // Create agent definition with Twitter tool
    let agent_def = AgentDefinition {
        name: "Twitter Agent".to_string(),
        description: "Agent that can access Twitter".to_string(),
        system_prompt: Some(SYSTEM_PROMPT.to_string()),
        model_settings: ModelSettings::default(),
        mcp_servers: vec![get_twitter_tool()],
        parameters: Default::default(),
    };
    return agent_def;
}
