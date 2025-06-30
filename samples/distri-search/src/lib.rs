use anyhow::Result;
use distri::{
    coordinator::{CoordinatorContext, LocalCoordinator, DISTRI_LOCAL_SERVER},
    servers::registry::{ServerMetadata, ServerRegistry, ServerTrait},
    store::{AgentStore, InMemoryAgentStore},
    types::{Configuration, TransportType},
};
use dotenv::dotenv;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub fn load_config() -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = include_str!("../deep-search.yaml");

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

pub async fn init_registry_and_coordinator(
    agent_store: Arc<dyn AgentStore>,
    context: Arc<CoordinatorContext>,
) -> (Arc<RwLock<ServerRegistry>>, Arc<LocalCoordinator>) {
    let server_registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let reg_clone = server_registry.clone();
    let mut registry = reg_clone.write().await;

    let coordinator = Arc::new(LocalCoordinator::new(
        server_registry.clone(),
        None,
        None,
        agent_store,
        context.clone(),
    ));

    registry.register(
        "web_search".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );
    registry.register(
        "crawl".to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_spider::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    let coordinator_clone = coordinator.clone();
    registry.register(
        DISTRI_LOCAL_SERVER.to_string(),
        ServerMetadata {
            auth_session_key: None,
            mcp_transport: TransportType::InMemory,
            kg_memory: None,
            builder: Some(Arc::new(move |_, transport| {
                let coordinator = coordinator.clone();
                let context = context.clone();
                let server = distri::coordinator::build_server(transport, coordinator, context)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            memories: HashMap::new(),
        },
    );

    (server_registry, coordinator_clone)
}

pub async fn init_infrastructure() -> Result<(Arc<RwLock<ServerRegistry>>, Arc<LocalCoordinator>)> {
    let context = Arc::new(CoordinatorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(agent_store.clone(), context).await;

    Ok((registry, coordinator))
}
