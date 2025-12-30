use std::sync::{atomic::AtomicBool, Arc};

use crate::{
    agent::ExecutorContext,
    tools::{context::to_tool_context, state::AgentExecutorState, ConsoleLogTool},
    types::ToolCall,
};
use distri_types::Tool;

// Note: distri_js_sandbox and rustyscript dependencies are disabled for now due to edition2024 compatibility issue
use distri_js_sandbox::{FunctionDefinition, JsExecutor, JsWorker, JsWorkerError, JsWorkerOptions};
use serde_json::Value;

#[derive(Clone)]
pub struct CodeExecutor {
    pub context: Arc<ExecutorContext>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub has_external_tools: Arc<AtomicBool>,
}

impl CodeExecutor {
    pub fn new(context: Arc<ExecutorContext>, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self {
            context,
            tools,
            has_external_tools: Arc::new(AtomicBool::new(false)),
        }
    }

    fn get_tool_def(&self, name: &str) -> Result<&Arc<dyn Tool>, JsWorkerError> {
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
        Ok(tool_def)
    }
}

#[async_trait::async_trait]
impl JsExecutor for CodeExecutor {
    fn execute_sync(
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
        let tool_def = self.get_tool_def(name)?;
        if !tool_def.is_sync() {
            return Err(JsWorkerError::Other(format!("Tool {} is not sync", name)));
        }

        let input = if args.len() > 1 {
            return Err(JsWorkerError::Other(
                "Too many arguments provided".to_string(),
            ));
        } else if args.len() == 1 {
            args.first().unwrap_or_default()
        } else {
            return Err(JsWorkerError::Other("No arguments provided".to_string()));
        };
        let tool_context = Arc::new(to_tool_context(&self.context));
        let result = tool_def.execute_sync(
            ToolCall {
                tool_call_id: uuid::Uuid::new_v4().to_string(),
                tool_name: name.to_string(),
                input: input.clone(),
            },
            tool_context,
        );
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(
                    "🔧 CodeExecutor: Tool '{}' execution failed, error: {:?}",
                    name,
                    e
                );
                return Ok(Value::String(e.to_string()));
            }
        };

        tracing::debug!("🔧 CodeExecutor: Tool '{}' execution successful", name,);

        // Convert Vec<Part> back to Value for JavaScript compatibility
        let value = if result.len() == 1 {
            match &result[0] {
                distri_types::Part::Data(data) => data.clone(),
                _ => serde_json::json!({"result": result}),
            }
        } else {
            serde_json::json!({"parts": result})
        };

        Ok(value)
    }
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
        let tool_def = self.get_tool_def(name)?;
        if tool_def.is_sync() {
            return Err(JsWorkerError::Other(format!("Tool {} is not async", name)));
        }
        let input = if args.len() > 1 {
            return Err(JsWorkerError::Other("No arguments provided".to_string()));
        } else if args.len() == 1 {
            args.first().unwrap_or_default()
        } else {
            return Err(JsWorkerError::Other(
                "Too many arguments provided".to_string(),
            ));
        };

        let toolcall = ToolCall {
            tool_call_id: uuid::Uuid::new_v4().to_string(),
            tool_name: name.to_string(),
            input: input.clone(),
        };

        // Emit tool call event
        self.context
            .emit(distri_types::AgentEventType::ToolCalls {
                step_id: self.context.get_current_step_id().await.unwrap_or_default(),
                parent_message_id: self.context.get_current_message_id().await,
                tool_calls: vec![toolcall.clone()],
            })
            .await;
        if tool_def.is_external() {
            self.context
                .update_status(crate::types::TaskStatus::InputRequired)
                .await;
            tracing::debug!("🔧 CodeExecutor: Tool '{}' is external", name);
            self.has_external_tools
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(Value::Null)
        } else {
            // Check if tool needs ExecutorContext
            let result = if tool_def.needs_executor_context() {
                // Use unified tool execution function
                use crate::tools::execute_tool_with_executor_context;
                execute_tool_with_executor_context(
                    tool_def.as_ref(),
                    toolcall,
                    self.context.clone(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
            } else {
                // Handle regular tools with ToolContext
                let tool_context = Arc::new(to_tool_context(&self.context));
                tool_def.execute(toolcall, tool_context).await
            };

            match result {
                Ok(result) => {
                    tracing::debug!("🔧 CodeExecutor: Tool '{}' execution successful", name);

                    // Convert Vec<Part> back to Value for JavaScript compatibility
                    let value = if result.len() == 1 {
                        match &result[0] {
                            distri_types::Part::Data(data) => data.clone(),
                            _ => serde_json::json!({"result": result}),
                        }
                    } else {
                        serde_json::json!({"parts": result})
                    };

                    Ok(value)
                }
                Err(e) => {
                    tracing::error!(
                        "🔧 CodeExecutor: Tool '{}' execution failed, error: {}",
                        name,
                        e
                    );
                    Ok(Value::String(e.to_string()))
                }
            }
        }
    }
}

/// Execute code with tool injection
pub async fn execute_code_with_tools(
    code: &str,
    context: Arc<ExecutorContext>,
) -> Result<(Value, Vec<String>, bool), JsWorkerError> {
    // Get ALL tools from context (including MCP tools like search)
    let mut all_tools = context.get_tools().await;

    let code_state = Arc::new(AgentExecutorState::default());
    // Remove existing console_log and final_answer tools to avoid duplicates
    all_tools.retain(|tool| {
        let name = tool.get_name();
        name != "console_log"
    });

    let has_external_tools = Arc::new(AtomicBool::new(false));
    // Add state-aware versions of console_log and final_answer tools
    all_tools.push(Arc::new(ConsoleLogTool(code_state.clone())) as Arc<dyn Tool>);

    let functions = all_tools.iter().map(to_function_definition).collect();

    let executor = CodeExecutor::new(context, all_tools);

    let append_console = "globalThis.console = {log: rustyscript.functions['console_log']}";
    let wrapped_code = format!("{}\n {}", append_console, code);
    let worker = JsWorker::new(JsWorkerOptions {
        timeout: std::time::Duration::from_secs(20), // Reduced timeout
        functions,
        executor: Arc::new(executor),
    })
    .map_err(|e| {
        tracing::error!("Failed to create JS worker: {}", e);
        JsWorkerError::JsError(e.to_string())
    })?;

    tracing::debug!("Executing code: {}", wrapped_code);
    let result = worker.execute(&wrapped_code).map_err(|e| {
        tracing::error!("JS execution failed: {}", e);
        tracing::error!("Code being executed: {}", wrapped_code);
        JsWorkerError::Other(format!("Code execution failed: {}", e))
    })?;
    let has_external_tools = has_external_tools.load(std::sync::atomic::Ordering::Relaxed);

    let observations = code_state.get_observations().unwrap_or_default();
    tracing::debug!("JS execution result: {:?}", result);
    Ok((result, observations, has_external_tools))
}

fn to_function_definition(tool: &Arc<dyn Tool>) -> FunctionDefinition {
    // special sync functions;

    FunctionDefinition {
        name: tool.get_name(),
        description: Some(tool.get_description()),
        parameters: serde_json::json!({}),
        is_async: !tool.is_sync(),
    }
}
