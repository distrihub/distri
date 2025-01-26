use std::sync::Arc;

use agents::servers::{registry::ServerRegistry, tavily};
use mcp_sdk::transport::ServerAsyncTransport;

pub fn init_registry() -> Arc<ServerRegistry> {
    let server_registry = ServerRegistry::new();
    let mut registry = server_registry;

    registry.register::<ServerAsyncTransport, _>("twitter".to_string(), twitter_mcp::build);

    registry.register::<ServerAsyncTransport, _>("web_search".to_string(), tavily::build);

    Arc::new(registry)
}
