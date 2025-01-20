use std::time::Duration;

use anyhow::Result;

use mcp_sdk::{
    protocol::RequestOptions,
    transport::{ClientAsyncTransport, ServerAsyncTransport, Transport},
};
use serde_json::json;
use tracing::info;
use twitter_mcp::build;

#[tokio::main]
async fn main() -> Result<()> {
    let r = run().await;
    r
}

async fn async_server(transport: ServerAsyncTransport) {
    let server = build(transport.clone()).unwrap();
    server.listen().await.unwrap();
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    // Create transports
    let client_transport = ClientAsyncTransport::new(|t| tokio::spawn(async_server(t)));
    client_transport.open().await?;

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

    Ok(())
}
