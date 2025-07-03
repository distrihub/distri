use anyhow::Result;
use clap::Parser;
use distri_cli::{load_config, load_config_from_str, run_agent, EmbeddedCli};
use twitter_bot::{get_agent_server, init_agent_executor};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = EmbeddedCli::parse();

    let config = if let Some(config) = cli.config {
        load_config(config)?
    } else {
        let config_str = include_str!("../../definition.yaml");
        load_config_from_str(&config_str)?
    };

    let executor = init_agent_executor(&config).await?;
    let server = get_agent_server();
    run_agent(executor, server, cli.command, &config).await?;
    Ok(())
}
