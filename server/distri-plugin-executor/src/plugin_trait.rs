use anyhow::Result;

use distri_types::configuration::DistriConfiguration;
use distri_types::ToolCall;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Plugin execution context
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginContext {
    pub call_id: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub user_id: Option<String>,
    pub params: serde_json::Value,
    pub secrets: std::collections::HashMap<String, String>,
    pub auth_session: Option<serde_json::Value>, // AuthSession as JSON for cross-language compatibility
}

/// Standard plugin information structure that works for both WASM and TypeScript
/// Uses distri-types::integration::PluginData directly
pub type PluginInfo = distri_types::integration::PluginData;

/// Re-export tool definition as PluginItem for compatibility
pub type PluginItem = distri_types::integration::IntegrationToolDefinition;

#[async_trait::async_trait]
pub trait PluginFileResolver: Send + Sync {
    /// Read raw bytes for a plugin-relative path (e.g. "src/index.ts")
    fn read(&self, path: &str) -> anyhow::Result<Vec<u8>>;
}

#[derive(Clone)]
pub struct PluginLoadContext {
    pub package_name: String,
    pub entrypoint: Option<String>,
    pub manifest: DistriConfiguration,
    pub resolver: Arc<dyn PluginFileResolver>,
}

/// Unified trait for plugin executors (WASM and TypeScript)
#[async_trait::async_trait]
pub trait PluginExecutor: Send + Sync {
    /// Enable downcasting to concrete types
    fn as_any(&self) -> &dyn std::any::Any;
    /// Load a plugin from its package path
    async fn load_plugin(&self, context: PluginLoadContext) -> Result<String>;

    /// Get plugin information (tools, workflows, etc.)
    async fn get_plugin_info(&self, package_name: &str) -> Result<PluginInfo>;

    /// Execute a tool (new concrete method)
    async fn execute_tool(
        &self,
        package_name: &str,
        tool_call: &ToolCall,
        context: PluginContext,
    ) -> Result<Value>;

    /// Execute a workflow (new concrete method)
    async fn execute_workflow(
        &self,
        package_name: &str,
        workflow_name: &str,
        input: Value,
        context: PluginContext,
    ) -> Result<Value>;

    /// Get list of loaded plugin names
    fn get_loaded_plugins(&self) -> Vec<String>;

    fn cleanup(&self);
}
