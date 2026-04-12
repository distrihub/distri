use anyhow::Result;
use clap::Parser;
use distri_server::agent_server::DistriAgentServer;
use distri_server_cli::{init_orchestrator, logging, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let level = std::env::var("DISTRI_LOG").unwrap_or_else(|_| "info".to_string());
    logging::init_logging(&level);

    dotenv::dotenv().ok();

    let cli = Cli::parse();

    if cli.verbose {
        distri_core::logging::init_diesel_instrumentation();
    }

    let workspace_path = distri_server_cli::workspace::resolve_workspace_path();

    // Initialize orchestrator
    let orchestrator = init_orchestrator(&workspace_path, &workspace_path).await?;

    let server_config = distri_types::configuration::ServerConfig {
        base_url: format!("http://{}:{}/v1", cli.host, cli.port),
        ..Default::default()
    };

    tracing::info!(
        "Starting Distri server at http://{}:{}/",
        cli.host,
        cli.port
    );

    DistriAgentServer::default()
        .start(
            server_config,
            orchestrator,
            Some(cli.host),
            Some(cli.port),
            cli.verbose,
        )
        .await
}
