use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{
    agent::AgentExecutor,
    types::{Configuration, ServerConfig},
    HashMapTaskStore, TaskStore,
};
use std::sync::Arc;
use tokio::sync::broadcast;

pub mod routes;
pub mod server;

pub mod agent_server;

#[cfg(test)]
mod tests {
    mod well_known_test;
}

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
    pub async fn initialize(
        config: &Configuration,
        executor: Arc<AgentExecutor>,
    ) -> anyhow::Result<Self> {
        let server_config = config.server.clone().unwrap_or_default();

        Ok(Self {
            executor,
            server_config,
        })
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

/// Starts the HTTP server (for backward compatibility)
pub async fn start_server(host: &str, port: u16) -> Result<()> {
    HttpServer::new(|| App::new().configure(routes::config))
        .bind((host, port))?
        .run()
        .await?;

    Ok(())
}
