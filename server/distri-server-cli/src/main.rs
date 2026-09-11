use anyhow::Result;
use clap::Parser;
use distri_server::agent_server::DistriAgentServer;
use distri_server_cli::{distri_yaml, init_orchestrator, logging, Cli};

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

    // `distri.yaml` is this deployment's settings file — the standalone
    // equivalent of cloud workspace settings. Read it before anything binds
    // a port: auth resolution below must be able to stop the boot.
    let mut distri_config = distri_yaml::load(&workspace_path)?.unwrap_or_default();
    let server_section = distri_config.server.take().unwrap_or_default();

    // Fail closed. Being told to authenticate and then starting
    // unauthenticated is the one outcome that must not happen, so an
    // unusable secret stops the boot with the variable named.
    let auth = distri_yaml::resolve_auth(&distri_config.auth, |name| std::env::var(name).ok())
        .map_err(|e| anyhow::anyhow!("refusing to start — {e}"))?;

    // Precedence: CLI flag / env var, then distri.yaml, then the default.
    let host = cli
        .host
        .or(server_section.host)
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = cli.port.or(server_section.port).unwrap_or(8081);

    // Initialize orchestrator
    let orchestrator = init_orchestrator(&workspace_path, &workspace_path, &distri_config).await?;

    let server_config = distri_types::configuration::ServerConfig {
        base_url: server_section
            .base_url
            .unwrap_or_else(|| format!("http://{host}:{port}/v1")),
        ..Default::default()
    };

    tracing::info!("Starting Distri server at http://{host}:{port}/");

    DistriAgentServer::default()
        .with_auth(auth)
        .start(
            server_config,
            orchestrator,
            Some(host),
            Some(port),
            cli.verbose,
            cli.ui_dist,
        )
        .await
}
