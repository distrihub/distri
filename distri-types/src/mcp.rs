use std::sync::Arc;

use async_mcp::{
    server::Server,
    transport::{ServerInMemoryTransport, Transport},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

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

/// Workspace-registry MCP server entry.
///
/// This is the user-facing definition stored alongside other workspace settings
/// (and JSON-encoded in API payloads). It is pure data — no closures — so it
/// round-trips through serialization unchanged.
///
/// The `transport` field carries everything needed for the `rmcp` client to
/// connect: stdio command, streamable-http URL, or SSE URL. Secrets that the
/// process needs come from `env` (for stdio) or `headers` (for HTTP/SSE).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
pub struct McpServerConfig {
    /// Identifier referenced by `ToolsConfig.mcp[].server` in agent definitions.
    pub name: String,
    /// Short human-facing description shown in the UI list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Transport details — how to reach the server.
    pub transport: McpClientTransport,
    /// Optional connection name (from the connections registry) to pull an
    /// auth token from at runtime. The token is injected as a bearer header
    /// (for HTTP/SSE) or as an env var (for stdio, under `auth_env_var`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// When `connection` is set and transport is `Stdio`, name of the env var
    /// the spawned process expects to receive the token in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_env_var: Option<String>,
    /// When true, the server is enabled for tool resolution.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Client-side transport configuration for connecting to an MCP server.
///
/// Mirrors the variants the `rmcp` crate supports. `headers` and `env` are
/// always optional — most well-known public servers don't need them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpClientTransport {
    /// Spawn a child process; communicate over its stdio.
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    /// Streamable HTTP transport (MCP 2025-03-26+ spec). One bidirectional
    /// endpoint that may upgrade individual responses to SSE — this covers
    /// both modern Streamable HTTP servers and legacy SSE-only servers
    /// reachable via the same URL.
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
}

impl McpServerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("MCP server name must be non-empty".to_string());
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "MCP server name '{}' must be alphanumeric/underscore/dash only",
                self.name
            ));
        }
        match &self.transport {
            McpClientTransport::Stdio { command, .. } => {
                if command.trim().is_empty() {
                    return Err("stdio transport requires a command".to_string());
                }
            }
            McpClientTransport::StreamableHttp { url, .. } => {
                if url.trim().is_empty() {
                    return Err("http/sse transport requires a url".to_string());
                }
                url::Url::parse(url).map_err(|e| format!("invalid url '{}': {}", url, e))?;
            }
        }
        Ok(())
    }
}
