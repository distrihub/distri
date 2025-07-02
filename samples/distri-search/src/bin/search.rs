use anyhow::Result;
use clap::Parser;
use distri_cli::{load_config, Cli, Commands};
use distri_search::{get_server, init_executor, load_config as load_embedded_config};
use distri_server::reusable_server::{list_agents, run_cli, run_server};

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

    let executor = init_executor(&config).await?;

    let executor_clone = executor.clone();
    let executor_handle = tokio::spawn(async move { executor_clone.run().await.unwrap() });

    match cli.command {
        Commands::Run { agent, task } => run_cli(executor, &agent, &task).await,
        Commands::List {} => list_agents(executor).await,
        Commands::Serve { host, port } => {
            let server = get_server();
            run_server(server, executor, config, &host, port).await
        }
    }?;

    executor_handle.abort();
    Ok(())
}
