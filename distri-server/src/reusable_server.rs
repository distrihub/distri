use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri::agent::{AgentExecutor, ExecutorContext};
use distri::servers::registry::ServerMetadata;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{configure_distri_service, DistriServer, DistriServiceConfig};
use distri::types::Configuration;

/// Trait for customizing Distri server instances
pub trait DistriServerCustomizer: Send + Sync {
    /// Get service name for logging and responses
    fn service_name(&self) -> &str;

    /// Get service description
    fn service_description(&self) -> &str;

    /// Get service capabilities list
    fn service_capabilities(&self) -> Vec<String>;

    /// Configure additional actix-web routes (optional)
    fn configure_routes(&self, cfg: &mut web::ServiceConfig);

    /// Customize the servers to be registered
    fn custom_servers(&self) -> HashMap<String, ServerMetadata> {
        HashMap::new()
    }
}

/// Default implementation of DistriServerCustomizer
pub struct DefaultCustomServer {
    service_name: String,
    description: String,
    capabilities: Vec<String>,
}

impl DefaultCustomServer {
    pub fn new() -> Self {
        Self {
            service_name: "distri-server".to_string(),
            description: "A Distri server instance".to_string(),
            capabilities: vec!["agent_execution".to_string(), "task_management".to_string()],
        }
    }

    pub fn with_service_name(mut self, name: &str) -> Self {
        self.service_name = name.to_string();
        self
    }

    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<&str>) -> Self {
        self.capabilities = capabilities.into_iter().map(|s| s.to_string()).collect();
        self
    }
}

impl DistriServerCustomizer for DefaultCustomServer {
    fn service_name(&self) -> &str {
        &self.service_name
    }

    fn service_description(&self) -> &str {
        &self.description
    }

    fn service_capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }

    fn configure_routes(&self, cfg: &mut web::ServiceConfig) {
        // Default implementation adds no additional routes
        let _ = cfg;
    }

    fn custom_servers(&self) -> HashMap<String, ServerMetadata> {
        HashMap::new()
    }
}

/// Builder for creating reusable distri servers with customization
pub struct DistriServerBuilder {
    customizer: Box<dyn DistriServerCustomizer>,
}

impl DistriServerBuilder {
    pub fn new() -> Self {
        Self {
            customizer: Box::new(DefaultCustomServer::new()),
        }
    }

    pub fn with_customizer(mut self, customizer: Box<dyn DistriServerCustomizer>) -> Self {
        self.customizer = customizer;
        self
    }

    pub fn with_service_name(self, name: &str) -> Self {
        self.with_customizer(Box::new(DefaultCustomServer::new().with_service_name(name)))
    }

    pub fn with_description(self, description: &str) -> Self {
        self.with_customizer(Box::new(
            DefaultCustomServer::new().with_description(description),
        ))
    }

    pub fn with_capabilities(self, capabilities: Vec<&str>) -> Self {
        self.with_customizer(Box::new(
            DefaultCustomServer::new().with_capabilities(capabilities),
        ))
    }

    /// Start the server with the configured settings
    pub async fn start(self, config: Configuration, host: &str, port: u16) -> Result<()> {
        let service_name = self.customizer.service_name();

        tracing::info!("Starting {}...", service_name);
        tracing::info!("Starting server on http://{}:{}", host, port);
        tracing::info!("Try these endpoints:");
        tracing::info!("  - http://{}:{}/            - Welcome page", host, port);
        tracing::info!("  - http://{}:{}/health      - Health check", host, port);
        tracing::info!("  - http://{}:{}/api/v1/agents - List agents", host, port);

        // For now, we don't have direct access to the DistriServer's registry
        // The customizer pattern will be used more directly when we integrate
        // with the full agent system initialization

        // Initialize the DistriServer with the customized config
        let distri_server =
            DistriServer::initialize(&config, self.customizer.custom_servers()).await?;
        let executor = distri_server.executor();
        let server_config = config.server.unwrap_or_default();

        // Create and configure the HTTP server
        let customizer = Arc::new(self.customizer);
        HttpServer::new(move || {
            let executor = executor.clone();
            let customizer = customizer.clone();
            let service_name = customizer.service_name().to_string();
            let service_description = customizer.service_description().to_string();
            let service_capabilities = customizer.service_capabilities();

            let mut app = App::new()
                .wrap(Logger::default())
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header()
                        .max_age(3600),
                )
                .route(
                    "/",
                    web::get().to({
                        let service_name = service_name.clone();
                        let service_description = service_description.clone();
                        let service_capabilities = service_capabilities.clone();
                        move || {
                            let service_name = service_name.clone();
                            let service_description = service_description.clone();
                            let service_capabilities = service_capabilities.clone();
                            async move {
                                default_welcome(
                                    &service_name,
                                    &service_description,
                                    &service_capabilities,
                                )
                                .await
                            }
                        }
                    }),
                )
                .route(
                    "/health",
                    web::get().to({
                        let service_name = service_name.clone();
                        move || {
                            let service_name = service_name.clone();
                            async move { default_health_check(&service_name).await }
                        }
                    }),
                )
                .configure(|cfg| {
                    let config = DistriServiceConfig::new(executor.clone(), server_config.clone());
                    configure_distri_service(cfg, config);
                });

            // Allow customizer to add additional routes
            app = app.configure(|cfg| customizer.configure_routes(cfg));
            app
        })
        .bind((host, port))?
        .run()
        .await?;

        Ok(())
    }
}

async fn default_welcome(
    service_name: &str,
    description: &str,
    capabilities: &[String],
) -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "message": format!("Welcome to {}!", service_name),
        "description": description,
        "endpoints": {
            "health": "/health",
            "distri_api": "/api/v1/*"
        },
        "capabilities": capabilities
    })))
}

async fn default_health_check(service_name: &str) -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": service_name,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

/// Run CLI with the given configuration
pub async fn run_cli(executor: Arc<AgentExecutor>, agent: &str, task: &str) -> Result<()> {
    tracing::info!("Running Distri Search CLI...");

    let context = Arc::new(ExecutorContext::default());
    let task_step = distri::memory::TaskStep {
        task: task.to_string(),
        task_images: None,
    };

    let result = executor
        .execute(agent, task_step, None, context, None)
        .await
        .map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;

    println!("Search Result:\n{}", result);

    Ok(())
}

/// Run the distri-search server with customized registry
pub async fn run_server(
    config: Configuration,
    customizer: Box<dyn DistriServerCustomizer>,
    host: &str,
    port: u16,
) -> Result<()> {
    DistriServerBuilder::new()
        .with_customizer(customizer)
        .start(config, host, port)
        .await
}

/// List available agents
pub async fn list_agents(executor: Arc<AgentExecutor>) -> Result<()> {
    tracing::info!("Available Agents:");

    let (agents, _) = executor.agent_store.list(None, None).await;

    for agent in agents {
        let definition = agent.get_definition();
        println!("  - {}: {}", definition.name, definition.description);

        if let Some(prompt) = &definition.system_prompt {
            let preview = if prompt.len() > 100 {
                format!("{}...", &prompt[..97])
            } else {
                prompt.clone()
            };
            println!("    Description: {}", preview);
        }
    }

    Ok(())
}
