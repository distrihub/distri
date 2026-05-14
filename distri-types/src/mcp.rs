use std::sync::Arc;

use async_mcp::{
    server::Server,
    transport::{ServerInMemoryTransport, Transport},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::auth::AuthType;

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

/// Client-side transport configuration for connecting to an MCP server.
///
/// Mirrors the variants the `rmcp` crate supports. `headers` is always optional
/// — most public servers don't need extra headers, and OAuth bearers are
/// injected by the connection resolver at pool-connect time rather than being
/// stored here.
///
/// The `Stdio` variant is intentionally absent: connections are workspace
/// resources that must be portable across hosts, so spawning a local child
/// process is not user-configurable. In-process A2A servers use the separate
/// `TransportType` enum on `McpServerMetadata`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpClientTransport {
    /// Streamable HTTP transport (MCP 2025-03-26+ spec). One bidirectional
    /// endpoint that may upgrade individual responses to SSE.
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    /// Legacy SSE-only transport (MCP 2024-11-05 spec). Kept for servers that
    /// haven't migrated yet; prefer Streamable HTTP for new connections.
    Sse {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
}

impl McpClientTransport {
    pub fn url(&self) -> &str {
        match self {
            Self::StreamableHttp { url, .. } | Self::Sse { url, .. } => url.as_str(),
        }
    }

    pub fn headers(&self) -> Option<&HashMap<String, String>> {
        match self {
            Self::StreamableHttp { headers, .. } | Self::Sse { headers, .. } => headers.as_ref(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let url = self.url();
        if url.trim().is_empty() {
            return Err("transport requires a url".to_string());
        }
        url::Url::parse(url).map_err(|e| format!("invalid url '{}': {}", url, e))?;
        Ok(())
    }
}

/// A pool-ready handle for one MCP server: identifier + transport + already-
/// resolved authorization headers.
///
/// Built by the host (e.g. distri-cloud) by looking up `kind = Mcp` connections
/// in scope and resolving their `auth_type` into bearer headers. Passed into
/// `McpClientPool::new` so the pool itself never has to touch the connection
/// store.
#[derive(Debug, Clone)]
pub struct McpServerHandle {
    /// Stable name used by agents in `ToolsConfig.mcp[].server`.
    pub name: String,
    pub transport: McpClientTransport,
    /// Headers to merge into the transport at connect time. Typically contains
    /// `Authorization: Bearer …` if the backing connection is OAuth.
    pub resolved_headers: HashMap<String, String>,
    pub enabled: bool,
}

impl McpServerHandle {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("MCP server handle name must be non-empty".to_string());
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "MCP server handle name '{}' must be alphanumeric/underscore/dash only",
                self.name
            ));
        }
        self.transport.validate()
    }
}
