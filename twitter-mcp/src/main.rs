use rust_mcp_sdk::transport::{StdioTransport, TransportOptions};
use rust_mcp_sdk::server_runtime;

mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::init();

    let server_details = server::get_server_details();
    let transport = StdioTransport::new(TransportOptions::default())?;
    let handler = server::TwitterHandler;

    let server = server_runtime::create_server(server_details, transport, handler);
    server.start().await?;

    Ok(())
}
