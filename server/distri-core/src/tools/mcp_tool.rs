//! Adapter that surfaces an MCP-server-provided tool as a `Tool` /
//! `ExecutorContextTool` so the rest of the agent runtime sees it the same as
//! any built-in tool.
//!
//! Created at tool-resolution time from a `McpToolHandle` discovered through
//! `McpClientPool`. The adapter holds a reference to the same pool used for
//! discovery, so every `tools/call` reuses the live connection that
//! `McpClientPool::connect_named` cached during resolution. Tool building is
//! the single chokepoint that hands the pool out — the orchestrator's
//! `create_agent_from_config` path is the only place that asks the attached
//! `McpPoolProvider` for a pool, and that pool is then threaded through tool
//! resolution into every adapter it produces.

use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::servers::{McpClientPool, McpToolHandle};
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::tool::ToolContext;
use distri_types::{Part, ResourceLink, Tool};

#[derive(Clone)]
pub struct McpToolAdapter {
    handle: McpToolHandle,
    /// Public-facing tool name. Defaults to the remote name but the resolver
    /// may prefix it with the server name to disambiguate when two servers
    /// expose tools with identical names.
    exposed_name: String,
    /// The pool this adapter was created from. Reusing the same `Arc<Pool>`
    /// across all adapters built in one run means a single connection per
    /// remote server, shared by every tool from it.
    pool: Arc<McpClientPool>,
}

impl std::fmt::Debug for McpToolAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpToolAdapter")
            .field("handle", &self.handle)
            .field("exposed_name", &self.exposed_name)
            .finish()
    }
}

impl McpToolAdapter {
    pub fn new(handle: McpToolHandle, exposed_name: String, pool: Arc<McpClientPool>) -> Self {
        Self {
            handle,
            exposed_name,
            pool,
        }
    }

    pub fn server(&self) -> &str {
        &self.handle.server
    }

    pub fn remote_name(&self) -> &str {
        &self.handle.name
    }
}

#[async_trait::async_trait]
impl Tool for McpToolAdapter {
    fn get_name(&self) -> String {
        self.exposed_name.clone()
    }

    fn get_description(&self) -> String {
        self.handle.description.clone()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.handle.input_schema.clone()
    }

    fn is_mcp(&self) -> bool {
        true
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn is_external(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "McpToolAdapter requires ExecutorContext for execution"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for McpToolAdapter {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let client = self
            .pool
            .connect_named(&self.handle.server)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("connect '{}': {e}", self.handle.server)))?;

        let result = client
            .call_tool(&self.handle.name, tool_call.input.clone())
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!(
                    "calling '{}/{}': {e}",
                    self.handle.server, self.handle.name
                ))
            })?;

        if result.is_error {
            return Err(AgentError::ToolExecution(format!(
                "MCP tool '{}/{}' returned error: {}",
                self.handle.server, self.handle.name, result.text
            )));
        }

        let mut parts: Vec<Part> = Vec::new();
        if !result.text.is_empty() {
            parts.push(Part::Text(result.text.clone()));
        }
        parts.extend(extract_resource_links(&result.raw));
        if parts.is_empty() {
            parts.push(Part::Text(String::new()));
        }
        Ok(parts)
    }
}

/// Walk the raw `tools/call` response and pull out any MCP resource
/// references (per-content-item or top-level `_meta.ui`). Surfaces them as
/// `Part::ResourceLink` so chat hosts that understand MCP-Apps (distrijs,
/// Claude, ChatGPT) can render the `ui://` resource in a sandboxed iframe
/// instead of showing only the flattened text fallback.
fn extract_resource_links(raw: &serde_json::Value) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    if let Some(content) = raw.get("content").and_then(|v| v.as_array()) {
        for item in content {
            let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind != "resource" && kind != "resource_link" {
                continue;
            }
            let resource = item.get("resource").unwrap_or(item);
            let uri = resource
                .get("uri")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let Some(uri) = uri else { continue };
            let mime_type = resource
                .get("mimeType")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let text = resource
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let meta = item.get("_meta").cloned();
            out.push(Part::ResourceLink(ResourceLink {
                uri,
                mime_type,
                text,
                meta,
            }));
        }
    }

    // Top-level `_meta.ui.resourceUri` — promote to a synthetic ResourceLink
    // when the server uses the result-level meta convention but no
    // per-content-item resource entry. This is the shape Zippy's MCP server
    // emits for `zippy.compose`.
    let already_has = |uri: &str| {
        out.iter().any(|p| match p {
            Part::ResourceLink(r) => r.uri == uri,
            _ => false,
        })
    };
    if let Some(top_meta) = raw.get("_meta") {
        if let Some(ui_uri) = top_meta
            .get("ui")
            .and_then(|v| v.get("resourceUri"))
            .and_then(|v| v.as_str())
        {
            if !already_has(ui_uri) {
                out.push(Part::ResourceLink(ResourceLink {
                    uri: ui_uri.to_string(),
                    mime_type: Some("text/html;profile=mcp-app".to_string()),
                    text: None,
                    meta: Some(top_meta.clone()),
                }));
            }
        }
    }
    out
}
