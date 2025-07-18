use std::sync::Arc;

use crate::{
    tools::{Tool, ToolContext},
    types::ToolCall,
};

// Note: distri_js_sandbox and rustyscript dependencies are disabled for now due to edition2024 compatibility issue
use distri_js_sandbox::{FunctionDefinition, JsExecutor, JsWorker, JsWorkerError, JsWorkerOptions};
use serde_json::Value;

#[derive(Clone)]
pub struct CodeExecutor {
    pub tools_context: ToolContext,
    pub tools: Vec<Arc<dyn Tool>>,
}

impl CodeExecutor {
    pub fn new(tools_context: ToolContext, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self {
            tools_context,
            tools,
        }
    }
}
#[async_trait::async_trait]
impl JsExecutor for CodeExecutor {
    async fn execute(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, JsWorkerError> {
        tracing::debug!(
            "🔧 CodeExecutor: Executing tool '{}' with args: {:?}",
            name,
            args
        );

        // Handle tool calls by delegating to the actual tools
        let tool_def = self
            .tools
            .iter()
            .find(|t| t.get_name() == name)
            .ok_or_else(|| {
                let available_tools: Vec<_> = self.tools.iter().map(|t| t.get_name()).collect();
                tracing::error!(
                    "🔧 CodeExecutor: Tool '{}' not found. Available tools: {:?}",
                    name,
                    available_tools
                );
                JsWorkerError::Other(format!(
                    "Tool {} not found. Available tools: {:?}",
                    name, available_tools
                ))
            })?;

        let result = tool_def
            .execute(
                ToolCall {
                    tool_call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: name.to_string(),
                    input: args.into_iter().map(|arg| arg.to_string()).collect(),
                },
                self.tools_context.clone(),
            )
            .await
            .map_err(|e| JsWorkerError::Other(e.to_string()))?;

        tracing::debug!(
            "🔧 CodeExecutor: Tool '{}' execution successful, result: {:?}",
            name,
            result
        );
        Ok(result)
    }
}

pub struct CodeExecutionResult {
    pub result: Value,
    pub observations: Vec<Value>,
}

/// Execute Python-like code with tool injection (simplified version for demonstration)
pub async fn execute_code_with_tools(
    code: &str,
    context: ToolContext,
    tools: Vec<Arc<dyn Tool>>,
) -> Result<Value, JsWorkerError> {
    let functions = tools.iter().map(to_function_definition).collect();
    let executor = CodeExecutor::new(context, tools);

    let append_console = "globalThis.console = {log: rustyscript.async_functions['console_log']}";
    let wrapped_code = format!("{}\n {}", append_console, code);
    let worker = JsWorker::new(JsWorkerOptions {
        timeout: std::time::Duration::from_secs(10),
        functions,
        executor: Arc::new(executor),
    })
    .map_err(|e| JsWorkerError::JsError(e.to_string()))?;

    let result = worker
        .execute(&wrapped_code)
        .map_err(|e| JsWorkerError::Other(e.to_string()))?;
    Ok(result)
}

fn to_function_definition(tool: &Arc<dyn Tool>) -> FunctionDefinition {
    FunctionDefinition {
        name: tool.get_name(),
        description: Some(tool.get_description()),
        parameters: serde_json::json!({}),
        returns: None,
    }
}
