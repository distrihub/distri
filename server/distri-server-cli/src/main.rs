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

    // Load configuration
    let config = distri_server_cli::load_distri_config(&cli.config);
    let workspace_path = distri_server_cli::workspace::resolve_workspace_path();

    // Initialize orchestrator
    let orchestrator = init_orchestrator(&workspace_path, &workspace_path, config.as_ref()).await?;

    // Build server config from workspace config
    let server_config = config
        .as_ref()
        .and_then(|c| c.server.clone())
        .unwrap_or_default();

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
