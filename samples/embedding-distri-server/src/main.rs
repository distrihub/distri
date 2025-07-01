use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Result as ActixResult};
use anyhow::Result;
use distri_cli::{load_config, initialize_executor, Cli, Commands};
use distri_server::{configure_distri_service, DistriServer, DistriServiceConfig, reusable_server::DistriServerBuilder};
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

async fn run_cli(config: distri::types::Configuration, agent_name: &str, task: &str) -> Result<()> {
    tracing::info!("Running agent '{}' with task: {}", agent_name, task);
    
    // Initialize executor using the centralized function
    let executor = initialize_executor(&config).await?;
    
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

async fn list_agents(config: distri::types::Configuration) -> Result<()> {
    let executor = initialize_executor(&config).await?;
    let (agents, _) = executor.agent_store.list(None, None).await;
    
    println!("Available agents:");
    for agent in agents {
        println!("  - {}: {}", agent.get_name(), agent.get_description());
    }
    
    Ok(())
}

async fn run_server(config: distri::types::Configuration, host: &str, port: u16) -> Result<()> {
    DistriServerBuilder::new()
        .with_service_name("embedding-distri-server")
        .with_description("This server demonstrates how to embed distri-server in your own actix-web application")
        .with_capabilities(vec!["agent_execution", "task_management"])
        .start(config, host, port)
        .await
}
