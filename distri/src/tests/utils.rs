use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    coordinator::{CoordinatorContext, LocalCoordinator, DISTRI_LOCAL_SERVER},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    types::{PlanConfig, TransportType},
    AgentDefinition, McpDefinition, McpSession, ModelSettings, ToolSessionStore,
};

pub fn get_tools_session_store() -> Option<Arc<Box<dyn ToolSessionStore>>> {
    dotenv::dotenv().ok();
    let session_key = std::env::var("X_USER_SESSION").unwrap();
    // Create executor with static session store

    Some(Arc::new(
        Box::new(StaticSessionStore { session_key }) as Box<dyn ToolSessionStore>
    ))
}

pub struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl ToolSessionStore for StaticSessionStore {
    async fn get_session(
        &self,
        _tool_name: &str,
        _context: &CoordinatorContext,
    ) -> anyhow::Result<Option<McpSession>> {
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
        name: "twitter".to_string(),
        r#type: Default::default(),
    }
}

pub async fn get_registry() -> Arc<RwLock<ServerRegistry>> {
    let mut server_registry = ServerRegistry::new();

    server_registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
            memories: HashMap::new(),
        },
    );

    Arc::new(RwLock::new(server_registry))
}

pub async fn register_coordinator(
    registry: Arc<RwLock<ServerRegistry>>,
    coordinator: Arc<LocalCoordinator>,
) {
    let mut registry = registry.write().await;
    let context = Arc::new(CoordinatorContext::default());
    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let context = context.clone();
                let server = crate::coordinator::build_server(transport, coordinator, context)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );
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

pub fn get_twitter_summarizer(
    planning_interval: Option<i32>,
    max_iterations: Option<u32>,
    max_tokens: Option<u32>,
) -> AgentDefinition {
    // Create agent definition with Twitter tool

    AgentDefinition {
        name: "Twitter Agent".to_string(),
        description: "Agent that can access Twitter".to_string(),
        system_prompt: Some(SYSTEM_PROMPT.to_string()),
        model_settings: ModelSettings {
            max_iterations: max_iterations.unwrap_or(10),
            max_tokens: max_tokens.unwrap_or(1000),
            model: "openai/gpt-4.1".to_string(),
            ..Default::default()
        },
        mcp_servers: vec![get_twitter_tool()],
        parameters: Default::default(),
        response_format: None,
        history_size: None,
        plan: planning_interval.map(|i| PlanConfig {
            interval: i,
            max_iterations: Some(max_iterations.unwrap_or(10) as i32),
            model_settings: ModelSettings {
                model: "openai/gpt-4.1-mini".to_string(),
                ..Default::default()
            },
        }),
        icon_url: None,
        skills: None,
    }
}
