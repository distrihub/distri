use anyhow::Result;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;

use crate::Part;
use crate::{
    ToolCall, ToolDefinition, auth::AuthMetadata, events::AgentEvent, stores::SessionStore,
};

/// Tool execution context - lighter weight than ExecutorContext
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Agent ID executing the tool
    pub agent_id: String,
    /// Session ID for the current conversation
    pub session_id: String,
    /// Task ID for the current task
    pub task_id: String,
    /// Run ID for the current execution
    pub run_id: String,
    /// Thread ID for conversation grouping
    pub thread_id: String,
    /// User ID if available
    pub user_id: String,
    /// Session store for persistent state across tool calls
    pub session_store: Arc<dyn SessionStore>,
    /// Event sender for emitting events during tool execution
    pub event_tx: Option<Arc<mpsc::Sender<AgentEvent>>>,

    /// Additional metadata for the tool. Useful in direct inline agent invocation.
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Tool trait for implementing tools that can be called by agents
#[async_trait::async_trait]
pub trait Tool: Send + Sync + std::fmt::Debug + std::any::Any {
    fn get_name(&self) -> String;

    /// Get the tool definition for the LLM
    fn get_tool_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.get_name(),
            description: self.get_description(),
            parameters: self.get_parameters(),
            output_schema: None,
            examples: self.get_tool_examples(),
        }
    }

    fn get_parameters(&self) -> serde_json::Value;
    fn get_description(&self) -> String;

    fn get_tool_examples(&self) -> Option<String> {
        None
    }

    /// Check if this tool is external (handled by frontend)
    fn is_external(&self) -> bool {
        false // Default to false for built-in tools
    }

    /// Check if this tool is an MCP tool
    fn is_mcp(&self) -> bool {
        false // Default to false for built-in tools
    }

    fn is_sync(&self) -> bool {
        false // Default to false for built-in tools
    }

    fn is_final(&self) -> bool {
        false // Default to false for built-in tools
    }

    /// Check if this tool needs ExecutorContext instead of ToolContext
    fn needs_executor_context(&self) -> bool {
        false // Default to false - most tools use ToolContext
    }

    /// Get authentication metadata for this tool
    fn get_auth_metadata(&self) -> Option<Box<dyn AuthMetadata>> {
        None // Default to no authentication required
    }

    /// Get the plugin name this tool belongs to (nullable)
    /// If this returns Some, the tool is part of a plugin
    /// If None, the tool is standalone
    fn get_plugin_name(&self) -> Option<String> {
        None // Default to standalone tool
    }

    /// Execute the tool with given arguments, returning content parts
    async fn execute(
        &self,
        tool_call: ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error>;

    /// Synchronous execution of the tool, returning content parts (default unsupported)
    fn execute_sync(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Sync execution not supported"))
    }
}
