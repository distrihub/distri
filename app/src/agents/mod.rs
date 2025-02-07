use std::{collections::HashMap, sync::Arc};

use agents::{
    servers::{
        registry::{ServerMetadata, ServerRegistry, ServerTrait},
        tavily,
    },
    types::TransportType,
};

pub fn _init_registry() -> Arc<ServerRegistry> {
    let server_registry = ServerRegistry::new();
    let mut registry = server_registry;

    registry.register(
        "twitter".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Async,
            builder: Some(Arc::new(|_, transport| {
                let server = twitter_mcp::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
            memories: HashMap::new(),
        },
    );

    registry.register(
        "web_search".to_string(),
        ServerMetadata {
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Async,
            builder: Some(Arc::new(|_, transport| {
                let server = tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
            kg_memory: None,
            memories: HashMap::new(),
        },
    );

    Arc::new(registry)
}
