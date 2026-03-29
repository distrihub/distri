use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::agent::todos::TodosTool;
use crate::agent::ExecutorContext;
use crate::servers::registry::McpServerRegistry;
use crate::tools::browser::{BrowserStepTool, DistriBrowserSharedTool, DistriScrapeSharedTool};
use crate::tools::builtin::ArtifactTool;
use crate::types::{ToolCall, ToolsConfig};
use crate::AgentError;
use distri_types::Part;
mod browser;
pub mod code;
// pub mod authenticated_example;
pub mod context;
pub mod shell;
mod state;
pub use code::execute_code_with_tools;
pub use context::to_tool_context;
mod builtin;
pub mod dynamic_factory;
pub mod inject_env;
pub mod request;
pub mod skill_script;
pub mod simulator;
pub mod tool_search;
pub mod resolve;
pub use builtin::{
    get_builtin_tools, AgentTool, ConsoleLogTool, DistriExecuteCodeTool, FinalTool,
    TransferToAgentTool,
};
pub use inject_env::InjectConnectionEnvTool;
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
    // Handle regular ExecutorContext tools via casting
    let executor_tool = cast_to_executor_context_tool(tool)?;
    let parts = executor_tool
        .execute_with_executor_context(tool_call, context)
        .await?;
    Ok(parts)
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
        // Code execution
        "distri_execute_code" => Ok(Box::new(DistriExecuteCodeTool)),
        // Tool discovery
        "tool_search" => Ok(Box::new(tool_search::ToolSearchTool)),
        // Connection env injection
        "inject_connection_env" => Ok(Box::new(inject_env::InjectConnectionEnvTool)),
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

/// Resolve tools from the new ToolsConfig format.
///
/// # Tool precedence (highest to lowest)
///
/// 1. **External tools** (from `ExecutorContext.dynamic_tools` or `DefinitionOverrides`)
///    — these are caller-provided and take highest priority. If an app overrides e.g.
///    `zippy_request` as an external tool, it replaces any dynamic factory tool of
///    the same name defined in the agent config.
/// 2. **Dynamic factory tools** (from `ToolsConfig.dynamic`) — agent-level factories
///    (e.g. `type = "http"` with `HttpFactoryConfig`). Skipped if an external tool
///    with the same name was already added.
/// 3. **Builtin tools** — server-provided tools (final, shell, search, etc.).
pub async fn resolve_tools_config(
    config: &ToolsConfig,
    _registry: Arc<RwLock<McpServerRegistry>>,
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

    // Add external tools — these take precedence over dynamic factory tools.
    // Collect their names so we can skip same-named factories below.
    let mut external_names = std::collections::HashSet::new();
    for tool_name in config.external.clone().unwrap_or(vec![]) {
        if tool_name == "*" {
            for tool in external_tools {
                external_names.insert(tool.get_name());
                all_tools.push(tool.clone());
            }
        } else if let Some(tool) = external_tools.iter().find(|t| t.get_name() == *tool_name) {
            external_names.insert(tool.get_name());
            all_tools.push(tool.clone());
        }
    }

    // Create dynamic factory tools — skip any whose name collides with an
    // external tool (external wins).
    for factory_def in &config.dynamic {
        if external_names.contains(&factory_def.name) {
            tracing::debug!(
                "Skipping dynamic factory tool '{}' — overridden by external tool",
                factory_def.name
            );
            continue;
        }
        match dynamic_factory::create_dynamic_tool(factory_def) {
            Ok(tool) => {
                // Wrap in DynExecutorTool so cast_to_executor_context_tool works via downcast
                all_tools.push(Arc::new(DynExecutorTool::new(tool)));
            }
            Err(e) => {
                tracing::warn!("Failed to create dynamic tool '{}': {}", factory_def.name, e)
            }
        }
    }

    // MCP tools are resolved at runtime via the MCP registry; no static resolution here.

    Ok(all_tools)
}

/// Simple glob-style pattern matching
/// Supports:
/// - "*" matches any sequence of characters
/// - "?" matches any single character
/// - Exact matches
#[cfg(test)]
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
            fn get_name(&self) -> String {
                "totally_unknown_tool".to_string()
            }
            fn get_description(&self) -> String {
                String::new()
            }
            fn get_parameters(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(
                &self,
                _: ToolCall,
                _: Arc<distri_types::ToolContext>,
            ) -> Result<Vec<Part>, anyhow::Error> {
                Ok(vec![])
            }
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

    // ── resolve_tools_config: external overrides dynamic factory ──

    /// Minimal mock tool for precedence tests.
    #[derive(Debug)]
    struct MockTool {
        name: String,
        description: String,
    }

    #[async_trait::async_trait]
    impl Tool for MockTool {
        fn get_name(&self) -> String {
            self.name.clone()
        }
        fn get_description(&self) -> String {
            self.description.clone()
        }
        fn get_parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _: ToolCall,
            _: Arc<distri_types::ToolContext>,
        ) -> Result<Vec<Part>, anyhow::Error> {
            Ok(vec![Part::Text(format!("executed:{}", self.name))])
        }
    }

    fn make_http_factory(name: &str) -> distri_types::dynamic_tool::DynamicToolFactory {
        distri_types::dynamic_tool::DynamicToolFactory {
            name: name.to_string(),
            factory_type: "http".to_string(),
            config: serde_json::json!({
                "base_url": "https://example.com",
                "headers": {}
            }),
            description: Some(format!("factory:{}", name)),
        }
    }

    /// Helper: run the external-vs-dynamic portion of resolve_tools_config
    /// without needing a real FileSystem. Tests just the precedence logic.
    async fn resolve_with_externals_and_factories(
        external_tools: &[Arc<dyn Tool>],
        external_config: Vec<String>,
        dynamic_factories: Vec<distri_types::dynamic_tool::DynamicToolFactory>,
    ) -> Vec<Arc<dyn Tool>> {
        // Collect external tool names (mirrors resolve_tools_config logic)
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        let mut external_names = std::collections::HashSet::new();

        for tool_name in &external_config {
            if tool_name == "*" {
                for tool in external_tools {
                    external_names.insert(tool.get_name());
                    tools.push(tool.clone());
                }
            } else if let Some(tool) =
                external_tools.iter().find(|t| t.get_name() == *tool_name)
            {
                external_names.insert(tool.get_name());
                tools.push(tool.clone());
            }
        }

        for factory_def in &dynamic_factories {
            if external_names.contains(&factory_def.name) {
                continue; // external wins
            }
            if let Ok(tool) = dynamic_factory::create_dynamic_tool(factory_def) {
                tools.push(Arc::new(DynExecutorTool::new(tool)));
            }
        }

        tools
    }

    #[tokio::test]
    async fn external_tool_overrides_dynamic_factory_with_same_name() {
        let external: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool {
            name: "zippy_request".to_string(),
            description: "external:zippy_request".to_string(),
        })];

        let tools = resolve_with_externals_and_factories(
            &external,
            vec!["*".to_string()],
            vec![make_http_factory("zippy_request")],
        )
        .await;

        let zippy: Vec<_> = tools.iter().filter(|t| t.get_name() == "zippy_request").collect();
        assert_eq!(zippy.len(), 1, "expected exactly 1 zippy_request, got {}", zippy.len());
        assert_eq!(
            zippy[0].get_description(),
            "external:zippy_request",
            "the external tool should win over the factory"
        );
    }

    #[tokio::test]
    async fn dynamic_factory_used_when_no_external_override() {
        let tools = resolve_with_externals_and_factories(
            &[],
            vec!["*".to_string()],
            vec![make_http_factory("zippy_request")],
        )
        .await;

        let zippy: Vec<_> = tools.iter().filter(|t| t.get_name() == "zippy_request").collect();
        assert_eq!(zippy.len(), 1, "factory tool should be present");
    }

    #[tokio::test]
    async fn external_override_only_affects_matching_name() {
        let external: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool {
            name: "zippy_request".to_string(),
            description: "external:zippy_request".to_string(),
        })];

        let tools = resolve_with_externals_and_factories(
            &external,
            vec!["*".to_string()],
            vec![
                make_http_factory("zippy_request"),
                make_http_factory("other_factory"),
            ],
        )
        .await;

        // zippy_request → external wins
        let zippy: Vec<_> = tools.iter().filter(|t| t.get_name() == "zippy_request").collect();
        assert_eq!(zippy.len(), 1);
        assert_eq!(zippy[0].get_description(), "external:zippy_request");

        // other_factory → factory survives (no external collision)
        let other: Vec<_> = tools.iter().filter(|t| t.get_name() == "other_factory").collect();
        assert_eq!(other.len(), 1, "non-colliding factory tool should remain");
    }
}
