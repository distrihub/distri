use actix_web::{web, App, HttpServer};
use anyhow::Result;
use distri::{
    agent::AgentExecutor,
    types::{Configuration, ServerConfig},
};
use std::sync::Arc;

pub mod agent_server;
pub mod handlers;
pub mod routes;
pub mod security;
pub mod server;

#[cfg(test)]
mod tests {
    mod well_known_test;
}

pub use server::A2AServer;
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

pub fn configure_distri_service(
    cfg: &mut web::ServiceConfig,
    executor: Arc<AgentExecutor>,
    server_config: ServerConfig,
) {
    cfg.app_data(web::Data::new(executor))
        .app_data(web::Data::new(server_config))
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
