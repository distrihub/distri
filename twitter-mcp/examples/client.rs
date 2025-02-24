use std::time::Duration;

use anyhow::Result;
use async_mcp::{
    client::ClientBuilder,
    protocol::RequestOptions,
    transport::{ClientStdioTransport, Transport},
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        // needs to be stderr due to stdio transport
        .with_writer(std::io::stderr)
        .init();

    // Create transport connected to cat command which will stay alive
    let transport = ClientStdioTransport::new("./mcp_test.sh", &[], None)?;

    // Open transport
    transport.open().await?;

    let client = ClientBuilder::new(transport).build();
    let client_clone = client.clone();
    tokio::spawn(async move { client_clone.start().await });

    let response = client
            .request(
                "tools/call",
                Some(json!({"name": "get_timeline", "arguments": {"session_string": "your_session_here"}})),
                RequestOptions::default().timeout(Duration::from_secs(10)),
            )
            .await?;
    println!("{:?}", response);

    Ok(())
}
