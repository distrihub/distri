use anyhow::Result;
use clap::Parser;
use distri_cli::{load_config, Cli, Commands};
use distri_search::{load_config as load_embedded_config, run_cli, run_server, list_agents};

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
            // Load configuration - prefer command line config, fallback to embedded
            let config = if !config.is_empty() {
                load_config(&config)?
            } else {
                load_embedded_config()?
            };
            
            if server {
                // Run as server
                run_server(config, &host, port).await
            } else {
                // Run as CLI
                run_cli(config, &agent, &task).await
            }
        }
        Commands::List { config } => {
            let config = if !config.is_empty() {
                load_config(&config)?
            } else {
                load_embedded_config()?
            };
            list_agents(config).await
        }
        Commands::Serve { config, host, port } => {
            let config = if !config.is_empty() {
                load_config(&config)?
            } else {
                load_embedded_config()?
            };
            run_server(config, &host, port).await
        }
    }
}
