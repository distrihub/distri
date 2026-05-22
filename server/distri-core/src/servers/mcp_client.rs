//! Client-side connections to remote MCP servers via the official `rmcp` SDK.
//!
//! This module owns the lifecycle of a single rmcp `RunningService<RoleClient, _>`
//! per `McpServerHandle`. A handle is a resolved view of a `kind = Mcp`
//! connection: its transport plus auth headers that the host already resolved
//! (e.g. `Authorization: Bearer …` from an OAuth connection). The pool never
//! reaches back into the connection store — auth happens upstream, the pool
//! just dials and dispatches.
//!
//! Two transports are supported, matching `McpClientTransport`:
//!   - `StreamableHttp` (single bidirectional HTTP endpoint, MCP 2025-03-26+ spec)
//!   - `Sse` (legacy Server-Sent-Events transport)

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use distri_types::{McpClientTransport, McpServerHandle};
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation, Tool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use tokio::sync::{Mutex, RwLock};

/// Lightweight handle describing one tool from a remote MCP server.
#[derive(Debug, Clone)]
pub struct McpToolHandle {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A connected MCP client. `Service` is the rmcp `RunningService` future-poller.
pub struct RemoteMcpClient {
    server_name: String,
    service: RunningService<RoleClient, ClientInfo>,
}

impl RemoteMcpClient {
    pub fn name(&self) -> &str {
        &self.server_name
    }

    /// Fetch the full tool list from the server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolHandle>> {
        let result = self
            .service
            .list_all_tools()
            .await
            .with_context(|| format!("listing tools on '{}'", self.server_name))?;
        Ok(result
            .into_iter()
            .map(|t: Tool| McpToolHandle {
                server: self.server_name.clone(),
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                input_schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect())
    }

    /// Invoke a tool by name with JSON arguments. Returns the assembled text
    /// payload from the MCP `content` array.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpCallResult> {
        let args_object = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(anyhow!(
                    "MCP tool arguments must be a JSON object, got {}",
                    other
                ))
            }
        };
        let mut params = CallToolRequestParams::new(tool_name.to_string());
        if let Some(args) = args_object {
            params = params.with_arguments(args);
        }
        let resp = self
            .service
            .call_tool(params)
            .await
            .with_context(|| format!("calling '{}/{}'", self.server_name, tool_name))?;

        let mut text = String::new();
        for item in &resp.content {
            if let Some(t) = item.as_text() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
        }
        Ok(McpCallResult {
            text,
            is_error: resp.is_error.unwrap_or(false),
            raw: serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null),
        })
    }

    /// Gracefully cancel the underlying connection.
    pub async fn shutdown(self) {
        let _ = self.service.cancel().await;
    }
}

#[derive(Debug, Clone)]
pub struct McpCallResult {
    pub text: String,
    pub is_error: bool,
    pub raw: serde_json::Value,
}

/// Build a client identity advertised to the remote server during initialize.
fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("distri", env!("CARGO_PKG_VERSION")),
    )
}

/// Establish a fresh connection. Callers usually go through `McpClientPool`.
///
/// The handle's `resolved_headers` are merged on top of any static headers in
/// the transport — host-resolved auth (typically `Authorization`) wins over
/// transport-default headers.
pub async fn connect(handle: &McpServerHandle) -> Result<RemoteMcpClient> {
    if !handle.enabled {
        return Err(anyhow!("MCP server '{}' is disabled", handle.name));
    }
    handle
        .validate()
        .map_err(|e| anyhow!("invalid mcp handle '{}': {}", handle.name, e))?;

    let info = client_info();
    let merged_headers = merged_headers(&handle.transport, &handle.resolved_headers);
    let url = handle.transport.url().to_string();

    let service = match &handle.transport {
        McpClientTransport::StreamableHttp { .. } => {
            let client = reqwest_client_with_headers(&merged_headers)?;
            let transport = StreamableHttpClientTransport::with_client(
                client,
                rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                    url,
                ),
            );
            info.serve(transport).await.with_context(|| {
                format!("initializing streamable-http MCP server '{}'", handle.name)
            })?
        }
        McpClientTransport::Sse { .. } => {
            // The streamable-http transport gracefully handles servers that
            // upgrade to SSE on the same endpoint. Routing SSE through the
            // streamable transport keeps a single code path.
            let client = reqwest_client_with_headers(&merged_headers)?;
            let transport = StreamableHttpClientTransport::with_client(
                client,
                rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                    url,
                ),
            );
            info.serve(transport)
                .await
                .with_context(|| format!("initializing sse MCP server '{}'", handle.name))?
        }
    };

    Ok(RemoteMcpClient {
        server_name: handle.name.clone(),
        service,
    })
}

fn merged_headers(
    transport: &McpClientTransport,
    extra: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = transport.headers().cloned().unwrap_or_default();
    for (k, v) in extra {
        out.insert(k.clone(), v.clone());
    }
    out
}

fn reqwest_client_with_headers(headers: &HashMap<String, String>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if !headers.is_empty() {
        let mut hdrs = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .with_context(|| format!("invalid header name '{}'", k))?;
            let value = reqwest::header::HeaderValue::from_str(v)
                .with_context(|| format!("invalid value for header '{}'", k))?;
            hdrs.insert(name, value);
        }
        builder = builder.default_headers(hdrs);
    }
    builder.build().context("building reqwest client")
}

/// Pool of live MCP client connections keyed by server name.
///
/// Connections are created on first use and shared via `Arc`. A pool can be
/// shared across the agent loop for the duration of a single `execute()` call.
pub struct McpClientPool {
    handles: HashMap<String, McpServerHandle>,
    clients: RwLock<HashMap<String, Arc<RemoteMcpClient>>>,
    connect_lock: Mutex<()>,
}

impl McpClientPool {
    pub fn new(handles: Vec<McpServerHandle>) -> Self {
        let handles = handles
            .into_iter()
            .filter(|h| h.enabled)
            .map(|h| (h.name.clone(), h))
            .collect();
        Self {
            handles,
            clients: RwLock::new(HashMap::new()),
            connect_lock: Mutex::new(()),
        }
    }

    pub fn server_names(&self) -> Vec<String> {
        self.handles.keys().cloned().collect()
    }

    pub fn get_handle(&self, name: &str) -> Option<&McpServerHandle> {
        self.handles.get(name)
    }

    /// Connect (or reuse) a named server.
    pub async fn connect_named(&self, name: &str) -> Result<Arc<RemoteMcpClient>> {
        if let Some(client) = self.clients.read().await.get(name).cloned() {
            return Ok(client);
        }
        let _g = self.connect_lock.lock().await;
        if let Some(client) = self.clients.read().await.get(name).cloned() {
            return Ok(client);
        }
        let handle = self
            .handles
            .get(name)
            .ok_or_else(|| anyhow!("MCP server '{}' not configured", name))?;
        let client = Arc::new(connect(handle).await?);
        self.clients
            .write()
            .await
            .insert(name.to_string(), client.clone());
        Ok(client)
    }

    /// Enumerate all tools across every configured server. Servers that fail
    /// to connect are logged and skipped — one broken integration shouldn't
    /// take the whole resolver down.
    pub async fn discover_all_tools(&self) -> Vec<McpToolHandle> {
        let mut out = Vec::new();
        for name in self.handles.keys() {
            match self.connect_named(name).await {
                Ok(client) => match client.list_tools().await {
                    Ok(tools) => out.extend(tools),
                    Err(e) => tracing::warn!(server = %name, error = ?e, "list_tools failed"),
                },
                Err(e) => tracing::warn!(server = %name, error = ?e, "MCP connect failed"),
            }
        }
        out
    }
}

impl Default for McpClientPool {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
