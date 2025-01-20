use std::time::Duration;

use anyhow::Result;

use mcp_sdk::{
    protocol::RequestOptions,
    transport::{ServerChannelTransport, Transport},
};
use serde_json::json;
use tracing::info;
use twitter_mcp::build;

#[tokio::main]
async fn main() -> Result<()> {
    let r = run().await;
    r
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    // Create transports
    let (server_transport, client_transport) = ServerChannelTransport::new_pair();

    // Open both transports

    client_transport.open().await?;

    // Start server in background first
    let server = build(server_transport.clone())?;
    let server_handle = tokio::spawn(async move { server.listen().await });

    // Give server time to initialize
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create and start client
    let client = mcp_sdk::client::ClientBuilder::new(client_transport.clone()).build();
    let client_clone = client.clone();
    let client_handle = tokio::spawn(async move { client_clone.start().await });

    // Make a request
    let response = client
        .request(
            "tools/call",
            Some(json!({"name": "get_timeline", "arguments": {"session_string": "your_session_here"}})),
            RequestOptions::default().timeout(Duration::from_secs(5)),
        )
        .await?;

    info!("Got response: {:#?}", response);

    // Cleanup in order
    drop(client); // Drop client first
    client_transport.close().await?;
    client_handle.abort();

    server_transport.close().await?;
    server_handle.abort();

    // Wait for tasks to complete
    // let _ = tokio::join!(client_handle, server_handle);

    Ok(())
}
