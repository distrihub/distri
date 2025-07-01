use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    store::{AgentStore, InMemoryAgentStore},
    types::{Configuration, TransportType},
};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
mod store;

pub fn load_config() -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = include_str!("../definition.yaml");

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

pub async fn init_registry_and_coordinator(
    agent_store: Arc<dyn AgentStore>,
    context: Arc<ExecutorContext>,
) -> (Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>) {
    let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let reg_clone = server_registry.clone();
    let mut registry = reg_clone.write().await;

    let coordinator = Arc::new(AgentExecutor::new(
        server_registry.clone(),
        store::get_tools_session_store(),
        None,
        agent_store,
        context.clone(),
    ));

    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::InMemory,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_twitter::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
            memories: HashMap::new(),
        },
    );

    (server_registry, coordinator)
}

pub async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>)> {
    let context = Arc::new(ExecutorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(agent_store.clone(), context).await;

    Ok((registry, coordinator))
}
