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

    // --emit-openapi: write spec to disk and exit without starting the server.
    if let Some(path) = &cli.emit_openapi {
        use distri_server::openapi::ServerApiDoc;
        use utoipa::OpenApi;
        let spec = ServerApiDoc::openapi();
        let yaml = serde_yaml::to_string(&spec)?;
        std::fs::write(path, yaml)?;
        println!("Wrote OpenAPI spec to {}", path.display());
        return Ok(());
    }

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
            cli.ui_dist,
        )
        .await
}
