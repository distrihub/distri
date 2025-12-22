use std::sync::Arc;

use async_mcp::{
    server::Server,
    transport::{ServerInMemoryTransport, Transport},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::AuthType;

#[async_trait::async_trait]
pub trait ServerTrait: Send + Sync {
    async fn listen(&self) -> anyhow::Result<()>;
}

pub type BuilderFn = dyn Fn(&ServerMetadataWrapper, ServerInMemoryTransport) -> anyhow::Result<Box<dyn ServerTrait>>
    + Send
    + Sync;

#[async_trait::async_trait]
impl<T: Transport> ServerTrait for Server<T> {
    async fn listen(&self) -> anyhow::Result<()> {
        self.listen().await
    }
}

#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerMetadataWrapper {
    pub server_metadata: McpServerMetadata,
    #[serde(skip)]
    pub builder: Option<Arc<BuilderFn>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "lowercase")]
pub enum TransportType {
    InMemory,
    SSE {
        server_url: String,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    WS {
        server_url: String,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        env_vars: Option<HashMap<String, String>>,
    },
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpServerMetadata {
    #[serde(default)]
    pub auth_session_key: Option<String>,
    #[serde(default = "default_transport_type", flatten)]
    pub mcp_transport: TransportType,
    #[serde(default)]
    pub auth_type: Option<AuthType>,
}

pub fn default_transport_type() -> TransportType {
    TransportType::InMemory
}

impl std::fmt::Debug for McpServerMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerMetadata")
            .field("auth_session_key", &self.auth_session_key)
            .field("mcp_transport", &self.mcp_transport)
            .field("auth_type", &self.auth_type)
            .finish()
    }
}
