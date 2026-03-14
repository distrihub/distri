use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::agent::todos::TodosTool;
use crate::agent::ExecutorContext;
use crate::servers::registry::McpServerRegistry;
use crate::tools::browser::{BrowserStepTool, DistriBrowserSharedTool, DistriScrapeSharedTool};
use crate::tools::builtin::ArtifactTool;
use crate::types::{McpDefinition, McpToolConfig, ToolCall, ToolsConfig};
use crate::AgentError;
use distri_types::Part;
mod browser;
pub mod code;
// pub mod authenticated_example;
pub mod context;
mod mcp;
pub mod shell;
mod state;
pub use code::execute_code_with_tools;
pub use context::to_tool_context;
pub use mcp::get_mcp_tools;
mod builtin;
pub mod platform;
pub mod skill_script;
pub mod tool_search;
pub use builtin::{
    get_builtin_tools, AgentTool, ConsoleLogTool, DistriExecuteCodeTool, FinalTool,
    TransferToAgentTool,
};
pub use tool_search::ToolSearchTool;

#[derive(Debug, Clone)]
pub struct DynExecutorTool {
    inner: Arc<dyn ExecutorContextTool>,
}

impl DynExecutorTool {
    pub fn new(inner: Arc<dyn ExecutorContextTool>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> Arc<dyn ExecutorContextTool> {
        self.inner.clone()
    }
}

// Re-export the Tool trait from distri-types
pub use distri_types::{Tool, ToolContext};

/// Extension trait for tools that need ExecutorContext access
#[async_trait::async_trait]
pub trait ExecutorContextTool: Tool {
    /// Execute the tool with ExecutorContext instead of ToolContext, returning content parts
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError>;

    /// Synchronous execution of the tool with ExecutorContext, returning content parts (default unsupported)
    fn execute_sync_with_executor_context(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        Err(AgentError::ToolExecution(
            "Sync execution not supported".to_string(),
        ))
    }
}

/// Unified tool execution function that handles both MCP and regular ExecutorContext tools
/// Returns a vector of content parts from tool execution
pub async fn execute_tool_with_executor_context(
    tool: &dyn Tool,
    tool_call: crate::types::ToolCall,
    context: Arc<crate::agent::ExecutorContext>,
) -> Result<Vec<Part>, AgentError> {
    let tool_name = tool.get_name();

    // Check if it's an MCP tool first
    if tool.is_mcp() {
        // Handle MCP tools through the proper execution path that includes panic recovery
        use std::any::Any;
        if let Some(mcp_tool) = (tool as &dyn Any).downcast_ref::<mcp::McpTool>() {
            // Use the static execute method that includes panic recovery mechanisms
            let orchestrator = context.get_orchestrator()?;
            let value = mcp::McpTool::execute(
                &tool_call,
                &mcp_tool.mcp_definition,
                orchestrator.mcp_registry.clone(),
                orchestrator.tool_auth_handler.clone(),
                context.clone(),
            )
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
            // Wrap JSON value into parts
            Ok(vec![Part::Data(value)])
        } else {
            Err(AgentError::ToolExecution(format!(
                "Failed to downcast MCP tool '{}'",
                tool_name
            )))
        }
    } else {
        // Handle regular ExecutorContext tools via casting
        let executor_tool = cast_to_executor_context_tool(tool)?;
        let parts = executor_tool
            .execute_with_executor_context(tool_call, context)
            .await?;
        Ok(parts)
    }
}

/// Cast a Tool to an ExecutorContextTool based on its name (for non-MCP tools)
pub fn cast_to_executor_context_tool(
    tool: &dyn Tool,
) -> Result<Box<dyn ExecutorContextTool>, AgentError> {
    let tool_name = tool.get_name();

    // Check if it's an MCP tool first
    if tool.is_mcp() {
        // MCP tools should use execute_tool_with_executor_context instead
        return Err(AgentError::ToolExecution(format!(
            "MCP tool '{}' should use execute_tool_with_executor_context instead",
            tool_name
        )));
    }

    use std::any::Any;
    if let Some(dyn_tool) = (tool as &dyn Any).downcast_ref::<DynExecutorTool>() {
        return Ok(Box::new(dyn_tool.clone()));
    }

    // Check hardcoded tool names
    match tool_name.as_str() {
        "final" => Ok(Box::new(FinalTool)),
        "transfer_to_agent" => Ok(Box::new(TransferToAgentTool)),
        "write_todos" => Ok(Box::new(TodosTool)),
        // Browsr tools
        "browsr_scrape" => Ok(Box::new(DistriScrapeSharedTool)),
        "browsr_browser" => Ok(Box::new(DistriBrowserSharedTool)),
        "browser_step" => Ok(Box::new(BrowserStepTool)),
        "artifact_tool" => Ok(Box::new(ArtifactTool)),
        // Shell execution tools
        "start_shell" => Ok(Box::new(shell::StartShellTool)),
        "execute_shell" => Ok(Box::new(shell::ExecuteShellTool)),
        "stop_shell" => Ok(Box::new(shell::StopShellTool)),
        "load_skill" => Ok(Box::new(skill_script::LoadSkillTool)),
        "run_skill_script" => Ok(Box::new(skill_script::RunSkillScriptTool)),
        // Code execution
        "distri_execute_code" => Ok(Box::new(DistriExecuteCodeTool)),
        // Tool discovery
        "tool_search" => Ok(Box::new(tool_search::ToolSearchTool)),
        // Platform management tools
        "list_agents" => Ok(Box::new(platform::ListAgentsTool)),
        "list_skills" => Ok(Box::new(platform::ListSkillsTool)),
        "create_skill" => Ok(Box::new(platform::CreateSkillTool)),
        "delete_skill" => Ok(Box::new(platform::DeleteSkillTool)),
        "write_to_storage" => Ok(Box::new(platform::WriteToStorageTool)),
        "read_from_storage" => Ok(Box::new(platform::ReadFromStorageTool)),
        name if name.starts_with("call_") => {
            let safe_agent_name = name.strip_prefix("call_").unwrap_or(name);
            // Convert double underscores back to slashes for package/agent names
            let agent_name = safe_agent_name.replace("__", "/");
            Ok(Box::new(AgentTool::new(agent_name)))
        }
        _ => Err(AgentError::ToolExecution(format!(
            "Tool '{}' cannot be cast to ExecutorContextTool",
            tool_name
        ))),
    }
}

pub const APPROVAL_REQUEST_TOOL_NAME: &str = "approval_request";

#[async_trait::async_trait]
impl Tool for DynExecutorTool {
    fn get_name(&self) -> String {
        self.inner.get_name()
    }

    fn get_description(&self) -> String {
        self.inner.get_description()
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.inner.get_parameters()
    }

    fn get_tool_examples(&self) -> Option<String> {
        self.inner.get_tool_examples()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_auth_metadata(&self) -> Option<Box<dyn distri_types::auth::AuthMetadata>> {
        self.inner.get_auth_metadata()
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<distri_types::tool::ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "DynExecutorTool requires ExecutorContext for execution",
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DynExecutorTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        self.inner
            .execute_with_executor_context(tool_call, context)
            .await
    }
}

/// Resolve tools from the new ToolsConfig format
/// Uses PluginToolLoader for dynamic tool loading (supports both OSS and Cloud plugins)
pub async fn resolve_tools_config(
    config: &ToolsConfig,
    registry: Arc<RwLock<McpServerRegistry>>,
    plugin_tool_loader: Option<&dyn distri_types::stores::PluginToolLoader>,
    workspace_filesystem: Arc<distri_filesystem::FileSystem>,
    session_filesystem: Arc<distri_filesystem::FileSystem>,
    include_filesystem_tools: bool,
    external_tools: &[Arc<dyn Tool>],
) -> Result<Vec<Arc<dyn Tool>>> {
    let mut all_tools = Vec::new();

    // Add all builtin tools (both required and user-configured)
    let builtin_tools = get_builtin_tools(
        workspace_filesystem,
        session_filesystem,
        include_filesystem_tools,
    );

    let use_all_builtins = config.builtin.iter().any(|name| name == "*");
    if use_all_builtins {
        // Wildcard: include all builtin tools
        all_tools.extend(builtin_tools.iter().cloned());
    } else {
        let mut require_tool_names = vec!["final"];
        for builtin_name in &config.builtin {
            if !require_tool_names.contains(&builtin_name.as_str()) {
                require_tool_names.push(builtin_name);
            }
        }
        for builtin_name in require_tool_names {
            if let Some(tool) = builtin_tools.iter().find(|t| t.get_name() == *builtin_name) {
                all_tools.push(tool.clone());
            }
        }
    }

    // Add external tools
    for tool_name in config.external.clone().unwrap_or(vec![]) {
        if tool_name == "*" {
            all_tools.extend(external_tools.to_vec());
        } else if let Some(tool) = external_tools.iter().find(|t| t.get_name() == *tool_name) {
            all_tools.push(tool.clone());
        }
    }

    // Add MCP tools with filtering
    for mcp_config in &config.mcp {
        let mcp_tools = get_filtered_mcp_tools(mcp_config, registry.clone()).await?;
        all_tools.extend(mcp_tools);
    }

    // Add plugin tools from packages configuration using the loader
    if let Some(loader) = plugin_tool_loader {
        for (package_name, tool_names) in &config.packages {
            // Check if package exists
            let has_package = loader.has_package(package_name).await.unwrap_or(false);
            if !has_package {
                let available_packages = loader.list_packages().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Package '{}' not found in plugin tools registry. Available packages: {:?}",
                    package_name,
                    available_packages
                ));
            }

            // Get tools for this package
            let package_tools = loader.get_package_tools(package_name).await?;
            let mut tools_added = 0;

            for tool_name in tool_names {
                if tool_name == "*" {
                    // Add all tools from this package
                    for tool in &package_tools {
                        all_tools.push(tool.clone());
                        tracing::debug!(
                            "Added tool {} from package {} (wildcard)",
                            tool.get_name(),
                            package_name
                        );
                    }
                    tools_added += package_tools.len();
                } else {
                    // Add specific tool by name
                    if let Some(tool) = package_tools.iter().find(|t| t.get_name() == *tool_name) {
                        all_tools.push(tool.clone());
                        tracing::debug!(
                            "Added tool {} from package {}",
                            tool_name,
                            package_name
                        );
                        tools_added += 1;
                    }
                }
            }

            // Assert that at least one tool was found
            if tools_added == 0 {
                let available_tool_names: Vec<String> =
                    package_tools.iter().map(|t| t.get_name()).collect();
                return Err(anyhow::anyhow!(
                    "No tools found for package '{}' with requested tools {:?}. Available tools in package: {:?}",
                    package_name,
                    tool_names,
                    available_tool_names
                ));
            }
        }
    } else if !config.packages.is_empty() {
        return Err(anyhow::anyhow!(
            "Plugin tool loader not configured but packages requested: {:?}",
            config.packages.keys().collect::<Vec<_>>()
        ));
    }

    Ok(all_tools)
}

/// Get MCP tools with include/exclude filtering
async fn get_filtered_mcp_tools(
    config: &McpToolConfig,
    registry: Arc<RwLock<McpServerRegistry>>,
) -> Result<Vec<Arc<dyn Tool>>> {
    // Create a temporary McpDefinition for compatibility with existing get_mcp_tools
    let mcp_def = McpDefinition {
        name: config.server.clone(),
        r#type: crate::types::McpServerType::Tool,
        filter: None,      // We'll do our own filtering
        auth_config: None, // No auth config for basic MCP tools
    };

    // Get all tools from the server
    let all_tools = get_mcp_tools(&[mcp_def], registry).await?;

    // Apply include/exclude filtering
    let mut filtered_tools = Vec::new();

    for tool in all_tools {
        let tool_name = tool.get_name();

        // Check include patterns
        let included = if config.include.is_empty() {
            true // If no include patterns, include by default
        } else {
            config
                .include
                .iter()
                .any(|pattern| matches_pattern(&tool_name, pattern))
        };

        // Check exclude patterns
        let excluded = config
            .exclude
            .iter()
            .any(|pattern| matches_pattern(&tool_name, pattern));

        if included && !excluded {
            filtered_tools.push(tool);
        }
    }

    Ok(filtered_tools)
}

/// Simple glob-style pattern matching
/// Supports:
/// - "*" matches any sequence of characters
/// - "?" matches any single character
/// - Exact matches
fn matches_pattern(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern == text {
        return true;
    }

    // Simple wildcard matching
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            return text.starts_with(prefix)
                && text.ends_with(suffix)
                && text.len() >= prefix.len() + suffix.len();
        }
    }

    false
}

/// Emit final result as a text message with is_final flag
pub async fn emit_final(
    tool_call: ToolCall,
    context: Arc<ExecutorContext>,
) -> Result<(), AgentError> {
    let result = match tool_call.input {
        serde_json::Value::Object(mut obj) => {
            if let Some(value) = obj.remove("input") {
                match value {
                    serde_json::Value::String(s) => serde_json::Value::String(s),
                    other => other,
                }
            } else {
                serde_json::Value::Object(obj)
            }
        }
        other => other,
    };

    // Mark the state as completed with final result
    context.set_final_result(Some(result)).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── cast_to_executor_context_tool ───────────────────────────

    #[test]
    fn cast_final_tool() {
        let tool = FinalTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_transfer_to_agent_tool() {
        let tool = TransferToAgentTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_start_shell_tool() {
        let tool = shell::StartShellTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_execute_shell_tool() {
        let tool = shell::ExecuteShellTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_stop_shell_tool() {
        let tool = shell::StopShellTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_distri_execute_code_tool() {
        let tool = DistriExecuteCodeTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_tool_search_tool() {
        let tool = ToolSearchTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_agent_tool_with_call_prefix() {
        // AgentTool is returned for any "call_*" tool name
        let tool = AgentTool::new("my_agent".to_string());
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_agent_tool_with_package_name() {
        let tool = AgentTool::new("pkg/agent".to_string());
        // Tool name becomes "call_pkg__agent"
        assert_eq!(tool.get_name(), "call_pkg__agent");
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_ok());
    }

    #[test]
    fn cast_unknown_tool_returns_error() {
        // Create a simple mock tool that returns an unknown name
        #[derive(Debug)]
        struct UnknownTool;
        #[async_trait::async_trait]
        impl Tool for UnknownTool {
            fn get_name(&self) -> String { "totally_unknown_tool".to_string() }
            fn get_description(&self) -> String { String::new() }
            fn get_parameters(&self) -> serde_json::Value { serde_json::json!({}) }
            async fn execute(
                &self, _: ToolCall, _: Arc<distri_types::ToolContext>,
            ) -> Result<Vec<Part>, anyhow::Error> { Ok(vec![]) }
        }
        let tool = UnknownTool;
        let result = cast_to_executor_context_tool(&tool);
        assert!(result.is_err());
    }

    // ── matches_pattern ─────────────────────────────────────────

    #[test]
    fn pattern_wildcard_matches_everything() {
        assert!(matches_pattern("anything", "*"));
    }

    #[test]
    fn pattern_exact_match() {
        assert!(matches_pattern("search", "search"));
    }

    #[test]
    fn pattern_exact_no_match() {
        assert!(!matches_pattern("search", "scrape"));
    }

    #[test]
    fn pattern_prefix_wildcard() {
        assert!(matches_pattern("browsr_search", "browsr_*"));
    }

    #[test]
    fn pattern_suffix_wildcard() {
        assert!(matches_pattern("my_search_tool", "*_tool"));
    }

    #[test]
    fn pattern_middle_wildcard() {
        assert!(matches_pattern("browsr_search_tool", "browsr_*_tool"));
    }

    #[test]
    fn pattern_wildcard_no_match() {
        assert!(!matches_pattern("other_tool", "browsr_*"));
    }

    // ── Tool trait implementations ────────────────────────────────

    #[test]
    fn shell_tools_need_executor_context() {
        assert!(shell::StartShellTool.needs_executor_context());
        assert!(shell::ExecuteShellTool.needs_executor_context());
        assert!(shell::StopShellTool.needs_executor_context());
    }

    #[test]
    fn final_tool_is_final() {
        assert!(FinalTool.is_final());
    }

    #[test]
    fn distri_execute_code_tool_name() {
        assert_eq!(DistriExecuteCodeTool.get_name(), "distri_execute_code");
        assert!(DistriExecuteCodeTool.needs_executor_context());
    }
}
