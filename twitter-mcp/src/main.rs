use anyhow::Result;
use async_mcp::transport::ServerStdioTransport;
use twitter_mcp::build;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // needs to be stderr due to stdio transport
        .with_writer(std::io::stderr)
        .init();

    let server = build(ServerStdioTransport)?;
    let server_handle = tokio::spawn(async move { server.listen().await });

    server_handle
        .await?
        .map_err(|e| anyhow::anyhow!("Server error: {:#?}", e))?;
    Ok(())
}
