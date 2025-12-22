use anyhow::Result;
use clap::Parser;
use distri_server_cli::{
    logging,
    multi_agent_cli::{MultiAgentCliBuilder, ServeContext, DEFAULT_SERVE_HOST, DEFAULT_SERVE_PORT},
    Cli, Commands,
};
use distri_server::agent_server::DistriAgentServer;

#[tokio::main]
async fn main() -> Result<()> {
    let level = std::env::var("DISTRI_LOG").unwrap_or_else(|_| "info".to_string());
    logging::init_logging(&level);

    #[cfg(feature = "otel")]
    distri_core::logging::init_tracer_provider();

    dotenv::dotenv().ok();

    let cli = parse_cli_with_default_serve();

    MultiAgentCliBuilder::new()
        .with_cli_parser(move || cli.clone())
        .with_server_runner(|ctx| async move {
            let ServeContext {
                server_config,
                executor,
                host,
                port,
                verbose,
                ..
            } = ctx;

            DistriAgentServer::default()
                .start(server_config, executor, Some(host), Some(port), verbose)
                .await
        })
        .run()
        .await
}

fn parse_cli_with_default_serve() -> Cli {
    let mut cli = Cli::parse();

    if cli.command.is_none() {
        let host = std::env::var("DISTRI_HOST").unwrap_or_else(|_| DEFAULT_SERVE_HOST.to_string());
        let port = std::env::var("DISTRI_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_SERVE_PORT);

        tracing::info!(
            "No command provided; starting distri server with UI at http://{}:{}/ui/",
            host,
            port
        );

        cli.command = Some(Commands::Serve {
            host: Some(host),
            port: Some(port),
            headless: false,
        });
    }

    cli
}
