use anyhow::Result;
use clap::Parser;
use distri_cli::{load_config, Cli, Commands};
use distri_server::reusable_server::{list_agents, run_cli, run_server};
use twitter_bot::{init_executor, load_config as load_embedded_config, TwitterBotCustomizer};
#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let config = if let Some(config) = cli.config {
        load_config(&config)?
    } else {
        load_embedded_config()?
    };

    let customizer = Box::new(TwitterBotCustomizer::new());
    let executor = init_executor(&config).await?;

    match cli.command {
        Commands::Run {
            agent,
            task,
            server,
            host,
            port,
        } => {
            // Load configuration - prefer command line config, fallback to embedded

            if server {
                // Run as server
                run_server(config, customizer, &host, port).await
            } else {
                // Run as CLI
                run_cli(executor, &agent, &task).await
            }
        }
        Commands::List {} => list_agents(executor).await,
        Commands::Serve { host, port } => run_server(config, customizer, &host, port).await,
    }
}
