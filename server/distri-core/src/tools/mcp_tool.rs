//! Adapter that surfaces an MCP-server-provided tool as a `Tool` /
//! `ExecutorContextTool` so the rest of the agent runtime sees it the same as
//! any built-in tool.
//!
//! Created at tool-resolution time from a `McpToolHandle` discovered through
//! `McpClientPool`. The adapter holds only metadata; the actual `tools/call`
//! happens through the pool stored in `ExecutorContext.mcp_client_pool` so
//! every execution shares a single live connection.

use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::servers::McpToolHandle;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::tool::ToolContext;
use distri_types::{Part, Tool};

#[derive(Debug, Clone)]
pub struct McpToolAdapter {
    handle: McpToolHandle,
    /// Public-facing tool name. Defaults to the remote name but the resolver
    /// may prefix it with the server name to disambiguate when two servers
    /// expose tools with identical names.
    exposed_name: String,
}

impl McpToolAdapter {
    pub fn new(handle: McpToolHandle, exposed_name: String) -> Self {
        Self {
            handle,
            exposed_name,
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
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let pool = context.mcp_client_pool.clone().ok_or_else(|| {
            AgentError::ToolExecution(format!(
                "MCP tool '{}' called but no MCP client pool is configured on the executor context",
                self.exposed_name
            ))
        })?;

        let client = pool
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
        Ok(vec![Part::Text(result.text)])
    }
}
