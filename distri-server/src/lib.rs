use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext},
    servers::registry::ServerRegistry,
    store::{InMemoryAgentStore, LocalSessionStore, SessionStore},
    types::{Configuration, ServerConfig},
    HashMapTaskStore, TaskStore,
};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

pub mod routes;
pub mod server;

pub use server::A2AServer;

/// Configuration for embedding distri in other actix-web apps
pub struct DistriServiceConfig {
    pub coordinator: Arc<AgentExecutor>,
    pub task_store: Arc<dyn TaskStore>,
    pub server_config: ServerConfig,
}

impl DistriServiceConfig {
    /// Create a new configuration with default task store
    pub fn new(coordinator: Arc<AgentExecutor>, server_config: ServerConfig) -> Self {
        Self {
            coordinator,
            task_store: Arc::new(HashMapTaskStore::new()),
            server_config,
        }
    }

    /// Create a new configuration with custom task store
    pub fn with_task_store(
        coordinator: Arc<AgentExecutor>,
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

/// DistriServer with access to the executor and initialize method
pub struct DistriServer {
    executor: Arc<AgentExecutor>,
    server_config: ServerConfig,
}

impl DistriServer {
    pub fn new(executor: Arc<AgentExecutor>, server_config: ServerConfig) -> Self {
        Self {
            executor,
            server_config,
        }
    }

    /// Initialize DistriServer from configuration
    pub async fn initialize(config: &Configuration) -> anyhow::Result<Self> {
        let executor = Arc::new(AgentExecutor::initialize(config).await?);
        let server_config = config.server.clone().unwrap_or_default();
        
        Ok(Self {
            executor,
            server_config,
        })
    }

    /// Initialize DistriServer from configuration string or file path
    pub async fn initialize_from_config(config_source: &str) -> anyhow::Result<Self> {
        let config = if std::path::Path::new(config_source).exists() {
            // It's a file path
            let config_str = std::fs::read_to_string(config_source)?;
            serde_yaml::from_str::<Configuration>(&config_str)?
        } else {
            // It's a config string
            serde_yaml::from_str::<Configuration>(config_source)?
        };

        Self::initialize(&config).await
    }

    /// Get access to the executor
    pub fn executor(&self) -> Arc<AgentExecutor> {
        self.executor.clone()
    }

    /// Start the server
    pub async fn start(&self, host: &str, port: u16) -> Result<()> {
        let server = A2AServer::new(self.executor.clone());
        server.start(host, port, self.server_config.clone()).await
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
    let (event_broadcaster, _) = broadcast::channel::<String>(1000);
    let agent_store = config.coordinator.agent_store.clone();

    cfg.app_data(web::Data::new(config.coordinator))
        .app_data(web::Data::new(agent_store))
        .app_data(web::Data::new(config.task_store))
        .app_data(web::Data::new(event_broadcaster))
        .app_data(web::Data::new(config.server_config))
        .configure(routes::config);
}

/// Simple helper to create a AgentExecutor from a YAML config file
/// This is a convenience function for quick setup
pub async fn create_coordinator_from_config(
    config_str: &str,
) -> Result<(Arc<AgentExecutor>, ServerConfig)> {
    let config: Configuration = serde_yaml::from_str(&config_str)?;

    let executor = AgentExecutor::initialize(&config).await?;
    let server_config = config.server.unwrap_or_default();

    Ok((Arc::new(executor), server_config))
}

/// Starts the HTTP server (for backward compatibility)
pub async fn start_server(host: &str, port: u16) -> Result<()> {
    HttpServer::new(|| App::new().configure(routes::config))
        .bind((host, port))?
        .run()
        .await?;

    Ok(())
}
