use async_mcp::{
    protocol::RequestOptions,
    transport::{ClientInMemoryTransport, ServerInMemoryTransport, Transport},
};
use serde_json::json;
use std::{env, time::Duration};

async fn async_server(transport: ServerInMemoryTransport) {
    let server = crate::build(transport.clone()).unwrap();
    server.listen().await.unwrap();
}

#[tokio::test]
async fn test_methods() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    dotenv::dotenv().ok();
    let session = env::var("X_USER_SESSION").unwrap();

    // Create transports
    let client_transport = ClientInMemoryTransport::new(|t| tokio::spawn(async_server(t)));
    client_transport.open().await?;

    // Create and start client
    let client = async_mcp::client::ClientBuilder::new(client_transport.clone()).build();
    let client_clone = client.clone();
    let _client_handle = tokio::spawn(async move { client_clone.start().await });

    // Timeline
    let response = client
        .request(
            "tools/call",
            Some(json!({"name": "get_timeline", "arguments": {"session_string": session}})),
            RequestOptions::default().timeout(Duration::from_secs(5)),
        )
        .await?;
    println!("Timeline\n{response}");

    // Trends
    let response = client
        .request(
            "tools/call",
            Some(json!({"name": "get_trends", "arguments": {"session_string": session}})),
            RequestOptions::default().timeout(Duration::from_secs(5)),
        )
        .await?;
    println!("Trends\n{response}");

    // Search
    let response = client
        .request(
            "tools/call",
            Some(json!({"name": "search_tweets", "arguments": {"session_string": session,"query": "rust programming",
            "max_tweets": 5,
            "mode": "top"}})),
            RequestOptions::default().timeout(Duration::from_secs(5)),
        )
        .await?;
    println!("Search\n{response}");

    Ok(())
}
