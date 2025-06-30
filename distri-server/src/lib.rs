use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{
    coordinator::{CoordinatorContext, LocalCoordinator}, 
    servers::registry::ServerRegistry,
    store::{InMemoryAgentStore, LocalSessionStore, SessionStore},
    types::{Configuration, ServerConfig}, 
    HashMapTaskStore, TaskStore
};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid;

pub mod routes;
pub mod server;

pub use server::A2AServer;

/// Configuration for embedding distri in other actix-web apps
pub struct DistriServiceConfig {
    pub coordinator: Arc<LocalCoordinator>,
    pub task_store: Arc<dyn TaskStore>,
    pub server_config: ServerConfig,
}

impl DistriServiceConfig {
    /// Create a new configuration with default task store
    pub fn new(coordinator: Arc<LocalCoordinator>, server_config: ServerConfig) -> Self {
        Self {
            coordinator,
            task_store: Arc::new(HashMapTaskStore::new()),
            server_config,
        }
    }

    /// Create a new configuration with custom task store
    pub fn with_task_store(
        coordinator: Arc<LocalCoordinator>,
        task_store: Arc<dyn TaskStore>,
        server_config: ServerConfig,
    ) -> Self {
        Self {
            coordinator,
            task_store,
            server_config,
        }
    }
}

/// Configure distri routes for embedding in another actix-web app
/// 
/// # Example
/// ```rust
/// use actix_web::{web, App, HttpServer};
/// use distri_server::{configure_distri_service, DistriServiceConfig};
/// 
/// let config = DistriServiceConfig::new(coordinator, server_config);
/// let app = App::new()
///     .configure(|cfg| configure_distri_service(cfg, config))
///     .route("/", web::get().to(|| async { "Hello World!" }));
/// ```
pub fn configure_distri_service(cfg: &mut web::ServiceConfig, config: DistriServiceConfig) {
    let (event_broadcaster, _) = broadcast::channel(1000);
    let agent_store = config.coordinator.agent_store.clone();

    cfg.app_data(web::Data::new(config.coordinator))
        .app_data(web::Data::new(agent_store))
        .app_data(web::Data::new(config.task_store))
        .app_data(web::Data::new(event_broadcaster))
        .app_data(web::Data::new(config.server_config))
        .configure(routes::config);
}

/// Simple helper to create a LocalCoordinator from a YAML config file
/// This is a convenience function for quick setup
pub async fn create_coordinator_from_config(config_path: &str) -> Result<(Arc<LocalCoordinator>, ServerConfig)> {
    use std::fs;
    use std::collections::HashMap;

    let config_str = fs::read_to_string(config_path)?;
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    
    // Create default components
    let registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let session_store: Option<Arc<Box<dyn SessionStore>>> = Some(Arc::new(
        Box::new(LocalSessionStore::new())
    ));
    let agent_store = Arc::new(InMemoryAgentStore::new());
    
    // Create context explicitly
    let context = Arc::new(CoordinatorContext::new(
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        true,
        None,
        HashMap::new(),
    ));
    
    // Create coordinator
    let coordinator = LocalCoordinator::new(
        registry,
        None, // tool_sessions
        session_store,
        agent_store,
        context,
    );

    // Register agents from config
    for agent_config in config.agents {
        let agent_record = distri::types::AgentRecord::Local(agent_config.definition);
        coordinator.register_agent(agent_record).await?;
    }

    // Get server config or use default
    let server_config = config.server.unwrap_or_default();
    
    Ok((Arc::new(coordinator), server_config))
}

/// Starts the HTTP server (for backward compatibility)
pub async fn start_server(host: &str, port: u16) -> Result<()> {
    HttpServer::new(|| App::new().configure(routes::config))
        .bind((host, port))?
        .run()
        .await?;

    Ok(())
}
