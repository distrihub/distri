use std::{fs, sync::Arc};
mod server;
mod types;

use anyhow::Result;
use async_mcp::run_http_server;
use clap::{Parser, Subcommand};
use server::McpProxy;
use tracing::info;
use types::ProxyServerConfig;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "proxy.yaml", global = true)]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all configured servers
    List,
    /// Run the proxy server
    Run,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // Load server configuration
    let config: ProxyServerConfig = serde_yaml::from_str(&fs::read_to_string(&cli.config)?)?;
    let config = Arc::new(config);
    info!("Loaded configuration from {}", cli.config);

    match cli.command {
        Commands::List => {
            println!("Configured servers:");
            for name in config.servers.keys() {
                println!("- {}", name);
            }
        }
        Commands::Run => {
            info!(
                "Starting proxy server with {} servers",
                config.servers.len()
            );
            let port = config.port;
            let proxy = McpProxy::initialize(config).await?;

            run_http_server(port, None, move |transport| {
                let proxy = proxy.clone();
                async move {
                    let server = proxy.build(transport).await?;
                    Ok(server)
                }
            })
            .await?;
        }
    }

    Ok(())
}
