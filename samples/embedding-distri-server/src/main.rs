use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri::{agent::AgentExecutor, types::Configuration};
use distri_cli::{Cli, Commands};
use distri_server::{configure_distri_service, DistriServer, DistriServiceConfig};
use clap::Parser;
use serde_json::json;

// Simple version without distri integration for now to demonstrate the pattern
// This will work while we resolve the distri compilation issues

// Custom route handlers for the host application
async fn health_check() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "status": "healthy",
        "service": "embedding-distri-server",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

async fn welcome() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "message": "Welcome to the Distri Embedding Example!",
        "description": "This server demonstrates how to embed distri-server in your own actix-web application",
        "endpoints": {
            "health": "/health",
            "distri_api": "/api/v1/*"
        }
    })))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    
    // Parse CLI arguments
    let cli = Cli::parse();
    
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    match cli.command {
        Commands::Run { config, agent, task, server, host, port } => {
            // Load configuration
            let config = load_config(&config)?;
            
            if server {
                // Run as server
                run_server(config, &host, port).await
            } else {
                // Run as CLI
                run_cli(config, &agent, &task).await
            }
        }
        Commands::List { config } => {
            let config = load_config(&config)?;
            list_agents(config).await
        }
        Commands::Serve { config, host, port } => {
            let config = load_config(&config)?;
            run_server(config, &host, port).await
        }
    }
}

/// Load configuration from file path with environment variable substitution
fn load_config(config_path: &str) -> Result<Configuration> {
    // Load .env file if it exists
    dotenv::dotenv().ok();

    // Read the config file
    let config_str = std::fs::read_to_string(config_path)?;

    // Replace environment variables in the config string
    let config_str = replace_env_vars(&config_str);

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

/// Replace environment variables in config string ({{ENV_VAR}} format)
fn replace_env_vars(content: &str) -> String {
    let mut result = content.to_string();

    // Find all patterns matching {{ENV_VAR}}
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();

    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let env_var_name = cap.get(1).unwrap().as_str();

        if let Ok(env_value) = std::env::var(env_var_name) {
            result = result.replace(full_match, &env_value);
        }
    }

    result
}

async fn run_cli(config: Configuration, agent_name: &str, task: &str) -> Result<()> {
    tracing::info!("Running agent '{}' with task: {}", agent_name, task);
    
    // Initialize executor
    let executor = AgentExecutor::initialize(&config).await?;
    
    // Find the agent
    let agent = executor.agent_store.get(agent_name).await
        .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_name))?;
    
    // Execute the task
    let context = std::sync::Arc::new(distri::agent::ExecutorContext::default());
    let task_step = distri::memory::TaskStep {
        task: task.to_string(),
        task_images: None,
    };
    
    let result = agent.invoke(task_step, None, context, None).await?;
    println!("Result: {}", result);
    
    Ok(())
}

async fn list_agents(config: Configuration) -> Result<()> {
    let executor = AgentExecutor::initialize(&config).await?;
    let (agents, _) = executor.agent_store.list(None, None).await;
    
    println!("Available agents:");
    for agent in agents {
        println!("  - {}: {}", agent.get_name(), agent.get_description());
    }
    
    Ok(())
}

async fn run_server(config: Configuration, host: &str, port: u16) -> Result<()> {
    tracing::info!("Starting Distri Embedding Example Server...");

    tracing::info!("Starting server on http://{}:{}", host, port);
    tracing::info!("Try these endpoints:");
    tracing::info!("  - http://{}:{}/            - Welcome page", host, port);
    tracing::info!("  - http://{}:{}/health      - Health check", host, port);
    tracing::info!("  - http://{}:{}/api/v1/agents - List agents", host, port);

    // Initialize the DistriServer 
    let distri_server = DistriServer::initialize(&config).await?;
    let executor = distri_server.executor();
    let server_config = config.server.unwrap_or_default();
    
    // Create and configure the HTTP server
    HttpServer::new(move || {
        let executor = executor.clone();
        App::new()
            .wrap(Logger::default())
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600),
            )
            .wrap(actix_web::middleware::Logger::default())
            // Custom routes for this host application
            .route("/", web::get().to(welcome))
            .route("/health", web::get().to(health_check))
            // Distri routes
            .configure(|cfg| {
                let config = DistriServiceConfig::new(executor.clone(), server_config.clone());
                configure_distri_service(cfg, config);
            })
    })
    .bind((host, port))?
    .run()
    .await?;

    Ok(())
}
