use distri_types::{MessageRole, Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

use crate::agent::todos::TodosTool;
use crate::tools::browser::{
    BrowserStepTool, CrawlTool, DistriBrowserSharedTool, DistriScrapeSharedTool, SearchTool,
};
use crate::tools::shell::{ExecuteShellTool, StartShellTool, StopShellTool};
use crate::{
    agent::{file::run_file_agent, ExecutorContext},
    tools::{emit_final, state::AgentExecutorState, ExecutorContextTool},
    types::{Message, ToolCall, ToolResponse},
    AgentError,
};

/// Returns the set of builtin tools available to all agents.
/// Filesystem tools are no longer included as builtins — they should be
/// provided as external tools by the client or accessed via shell commands.
pub fn get_builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(TransferToAgentTool) as Arc<dyn Tool>,
        Arc::new(FinalTool) as Arc<dyn Tool>,
        Arc::new(ReflectTool) as Arc<dyn Tool>,
        Arc::new(DistriScrapeSharedTool) as Arc<dyn Tool>,
        Arc::new(DistriBrowserSharedTool) as Arc<dyn Tool>,
        Arc::new(BrowserStepTool) as Arc<dyn Tool>,
        Arc::new(SearchTool) as Arc<dyn Tool>,
        Arc::new(CrawlTool) as Arc<dyn Tool>,
        Arc::new(TodosTool) as Arc<dyn Tool>,
        Arc::new(StartShellTool) as Arc<dyn Tool>,
        Arc::new(ExecuteShellTool) as Arc<dyn Tool>,
        Arc::new(StopShellTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::tool_search::ToolSearchTool) as Arc<dyn Tool>,
        Arc::new(DistriExecuteCodeTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::inject_env::InjectConnectionEnvTool) as Arc<dyn Tool>,
    ]
}

/// Final tool for code execution mode with state management
#[derive(Debug)]
pub struct FinalTool;

#[async_trait::async_trait]
impl Tool for FinalTool {
    fn get_name(&self) -> String {
        "final".to_string()
    }
    fn get_description(&self) -> String {
        "Indicate that the task is complete and provide the final result to the user".to_string()
    }
    fn is_final(&self) -> bool {
        true
    }
    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "string",
            "description": "The final result or answer to provide to the user"
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "FinalTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for FinalTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        emit_final(tool_call, context.clone()).await?;
        // No direct parts to return
        Ok(Vec::new())
    }
}

/// Reflection tool that the reflection agent uses to signal its decision
/// instead of relying on "Should Continue" string matching in the output text.
#[derive(Debug)]
pub struct ReflectTool;

#[async_trait::async_trait]
impl Tool for ReflectTool {
    fn get_name(&self) -> String {
        "reflect".to_string()
    }

    fn get_description(&self) -> String {
        "Report the reflection analysis result. Use this tool to indicate whether the agent should retry execution based on quality and completeness assessment.".to_string()
    }

    fn is_final(&self) -> bool {
        true
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "quality": {
                    "type": "string",
                    "enum": ["excellent", "good", "fair", "poor"],
                    "description": "Quality assessment of the execution"
                },
                "completeness": {
                    "type": "string",
                    "enum": ["complete", "partial", "incomplete"],
                    "description": "Completeness assessment of the execution"
                },
                "should_continue": {
                    "type": "boolean",
                    "description": "Whether the agent should retry execution. true = retry, false = done"
                },
                "reason": {
                    "type": "string",
                    "description": "Brief explanation of the reflection decision"
                }
            },
            "required": ["quality", "completeness", "should_continue"]
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "ReflectTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ReflectTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Store the reflection result as the final result so the agent loop can read it
        let result = tool_call.input.clone();
        context.set_final_result(Some(result.clone())).await;
        Ok(vec![Part::Data(result)])
    }
}

/// Implementation of the transfer_to_agent built-in tool
#[derive(Debug)]
pub struct TransferToAgentTool;

#[async_trait::async_trait]
impl Tool for TransferToAgentTool {
    fn get_name(&self) -> String {
        "transfer_to_agent".to_string()
    }
    fn get_description(&self) -> String {
        "Transfer control to another agent. The target agent takes over completely — your execution stops and their result becomes the final response.".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }
    fn get_parameters(&self) -> serde_json::Value {
        json!({
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "The name of the agent to transfer control to"
                        },
                        "message": {
                            "type": "string",
                            "description": "The task/message to pass to the target agent. If omitted, the original user message is forwarded."
                        },
                        "reason": {
                            "type": "string",
                            "description": "Optional reason for the transfer (for logging/UI)"
                        }
                    },
                    "required": ["agent_name"]
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "TransferToAgentTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for TransferToAgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let args = tool_call.input;
        let args: HashMap<String, Value> = serde_json::from_value(args)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid JSON: {}", e)))?;
        let target_agent = args
            .get("agent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing agent_name parameter".to_string()))?;

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract task/message to pass to target agent
        let task_text = args
            .get("message")
            .or_else(|| args.get("task"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let orchestrator = context.get_orchestrator()?;

        // Verify target agent exists
        if orchestrator.get_agent(target_agent).await.is_none() {
            return Err(AgentError::ToolExecution(format!(
                "Target agent '{}' not found",
                target_agent
            )));
        }

        // Emit handover event for UI display
        context
            .emit(crate::agent::types::AgentEventType::AgentHandover {
                from_agent: context.agent_id.clone(),
                to_agent: target_agent.to_string(),
                reason: reason.clone(),
            })
            .await;

        tracing::info!(
            "Agent handover: {} → {} (reason: {:?})",
            context.agent_id,
            target_agent,
            reason
        );

        // Build the task for the target agent.
        // Use the explicit message/task param, or try to get the last user message from history.
        let child_task = if let Some(task) = task_text {
            task
        } else {
            // Try to extract the original user message from message history
            let history = context
                .get_current_task_message_history()
                .await
                .unwrap_or_default();
            history
                .iter()
                .rev()
                .find(|m| m.role == distri_types::MessageRole::User)
                .and_then(|m| m.as_text().map(|t| t.to_string()))
                .unwrap_or_else(|| "Continue the task".to_string())
        };

        // Use continue_as (not new_task) — the target agent continues in the SAME
        // task context so it can see the parent's scratchpad/history. This is what
        // makes transfer different from call_: the target agent has full context.
        let child_context = context.continue_as(target_agent).await;

        let message = distri_types::Message {
            id: uuid::Uuid::new_v4().to_string(),
            name: None,
            parts: vec![Part::Text(child_task)],
            role: distri_types::MessageRole::User,
            created_at: chrono::Utc::now().timestamp_millis(),
            agent_id: None,
            parts_metadata: None,
        };

        let child_context_arc = Arc::new(child_context);
        let child_context_clone = child_context_arc.clone();

        // Execute target agent synchronously and wait for result
        let result = orchestrator
            .execute_stream(target_agent, message, child_context_arc, None)
            .await;

        let final_result = match result {
            Ok(invoke_result) => {
                let child_final = child_context_clone.get_final_result().await;
                child_final
                    .or(invoke_result.content.map(|c| Value::String(c)))
                    .unwrap_or_else(|| Value::String("Agent completed without response".into()))
            }
            Err(e) => {
                tracing::error!("Transfer to {} failed: {}", target_agent, e);
                Value::String(format!("Transfer failed: {}", e))
            }
        };

        // Set the target agent's result as OUR final result.
        // This causes the parent agent loop to see get_final_result().is_some()
        // and stop iterating — achieving a true handover.
        context.set_final_result(Some(final_result.clone())).await;

        Ok(vec![Part::Data(final_result)])
    }
}

#[derive(Debug)]
pub struct ConsoleLogTool(pub Arc<AgentExecutorState>);

#[async_trait::async_trait]
impl Tool for ConsoleLogTool {
    fn is_sync(&self) -> bool {
        true
    }

    fn get_name(&self) -> String {
        "console_log".to_string()
    }

    fn get_description(&self) -> String {
        "Log a message to the console".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
                    "type": "string",
                    "description": "The message to log to the console"
        })
    }
    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        return Err(anyhow::anyhow!("Async execution not supported"));
    }

    fn execute_sync(
        &self,
        tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        tracing::debug!(
            "🔧 ConsoleLogTool: Executing console.log for tool call: {:?}",
            tool_call
        );

        let value = match serde_json::to_string(&tool_call.input) {
            Ok(value) => value,
            Err(e) => {
                tracing::error!("Failed to convert input to string: {}", e);
                e.to_string()
            }
        };

        self.0
            .add_observation(value)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        Ok(vec![Part::Data(Value::Null)])
    }
}

/// Agent tool that delegates tasks to other agents
#[derive(Debug)]
pub struct AgentTool {
    agent_name: String,
}

impl AgentTool {
    pub fn new(agent_name: String) -> Self {
        Self { agent_name }
    }
}

#[async_trait::async_trait]
impl Tool for AgentTool {
    fn get_name(&self) -> String {
        // Replace forward slashes with double underscores to create valid function names
        let safe_agent_name = self.agent_name.replace('/', "__");
        format!("call_{}", safe_agent_name)
    }

    fn get_description(&self) -> String {
        format!("Delegate a task to the {} agent", self.agent_name)
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "string",
            "description": "The task to delegate to the agent"
        })
    }

    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "AgentTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for AgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let normalized_input = match tool_call.input {
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

        tracing::debug!("AgentTool received input: {}", normalized_input);

        let task = match normalized_input {
            serde_json::Value::String(s) => s,
            other => serde_json::to_string(&other).unwrap_or_else(|_| other.to_string()),
        };

        let orchestrator = context.get_orchestrator()?;

        // Check if this sub-agent should run in a remote sandbox (deepagent mode)
        let use_remote = orchestrator.background_runner.is_some()
            && orchestrator.broadcaster.is_some()
            && {
                let agent_def = orchestrator
                    .stores
                    .agent_store
                    .get(&self.agent_name)
                    .await;
                matches!(
                    agent_def,
                    Some(distri_types::configuration::AgentConfig::StandardAgent(ref def)) if def.deepagent
                )
            };

        if use_remote {
            // DeepAgent path: spawn in sandbox, subscribe to events, relay to parent
            let runner = orchestrator.background_runner.as_ref().unwrap();
            let broadcaster = orchestrator.broadcaster.as_ref().unwrap();
            let sub_task_id = uuid::Uuid::new_v4().to_string();
            let env_id = format!(
                "{}-{}",
                context.workspace_id.as_deref().unwrap_or("default"),
                self.agent_name
            );

            tracing::info!(
                "DeepAgent dispatch: spawning {} in sandbox (task_id={})",
                self.agent_name,
                sub_task_id
            );

            // Fire-and-forget: spawn container/in-process execution
            runner
                .spawn(
                    sub_task_id.clone(),
                    self.agent_name.clone(),
                    task,
                    context.user_id.clone(),
                    context.workspace_id.clone(),
                    Some(env_id),
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to spawn deepagent: {}", e))
                })?;

            // Subscribe to sub-task events, relay to parent event channel
            let mut stream = broadcaster
                .subscribe(&sub_task_id)
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!(
                        "Failed to subscribe to deepagent events: {}",
                        e
                    ))
                })?;

            let mut final_result: Option<Value> = None;

            use futures_util::StreamExt;
            while let Some(event) = stream.next().await {
                // Relay events to parent's event stream so the caller sees progress
                context.emit(event.event.clone()).await;

                // Check for completion
                match &event.event {
                    distri_types::AgentEventType::RunFinished { .. } => {
                        // Read result from task store
                        if let Ok(Some(task_data)) =
                            orchestrator.stores.task_store.get_task(&sub_task_id).await
                        {
                            let a2a_task: distri_a2a::Task = task_data.into();
                            if let Some(msg) = a2a_task.status.message {
                                let text: String = msg
                                    .parts
                                    .iter()
                                    .filter_map(|p| match p {
                                        distri_a2a::Part::Text(t) => Some(t.text.clone()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                if !text.is_empty() {
                                    final_result = Some(Value::String(text));
                                }
                            }
                        }
                        break;
                    }
                    distri_types::AgentEventType::RunError { message, .. } => {
                        final_result = Some(Value::String(format!("Deepagent error: {}", message)));
                        break;
                    }
                    _ => {}
                }
            }

            Ok(vec![Part::Data(
                final_result.unwrap_or_else(|| Value::String("Deepagent completed".to_string())),
            )])
        } else {
            // Standard in-process path (existing behavior)
            // Create a new task context for the child agent (same thread, new task)
            let child_context = context.new_task(&self.agent_name).await;

            // Create message for the child agent
            let message = Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![Part::Text(task.clone())],
                role: MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let child_context_arc = Arc::new(child_context);
            let child_context_clone = child_context_arc.clone();

            let result = orchestrator
                .execute_stream(&self.agent_name, message, child_context_arc, None)
                .await;

            // Return result from child agent
            match result {
                Ok(invoke_result) => {
                    let final_result = child_context_clone.get_final_result().await;
                    let response_text = final_result
                        .or(invoke_result.content.map(|c| Value::String(c)))
                        .unwrap_or_else(|| {
                            "Child agent completed without response".to_string().into()
                        });

                    Ok(vec![Part::Data(response_text)])
                }
                Err(e) => Ok(vec![Part::Data(Value::String(e.to_string()))]),
            }
        }
    }
}

/// Artifact tool that allows agents to analyze their own created files
#[derive(Debug)]
pub struct ArtifactTool;

#[async_trait::async_trait]
impl Tool for ArtifactTool {
    fn get_name(&self) -> String {
        "artifact_tool".to_string()
    }

    fn get_description(&self) -> String {
        "Look through available artifacts (files that were automatically saved from previous tool responses) and return any meaningful information related to the given topic. The tool will discover and analyze relevant artifacts automatically - do not specify filenames.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "string",
            "description": "Topic or question to search for in available artifacts. Example: 'top millionaires list' or 'search results about billionaires'. The tool will automatically discover relevant artifacts and extract meaningful information."
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "ArtifactTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ArtifactTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let task = tool_call.input.as_str().ok_or_else(|| {
            AgentError::ToolExecution("Task parameter must be a string".to_string())
        })?;

        // Get orchestrator
        let orchestrator = context.get_orchestrator()?;

        // Create a tool response to pass to run_file_agent
        let tool_response = ToolResponse {
            tool_call_id: tool_call.tool_call_id.clone(),
            tool_name: "artifact_tool".to_string(),
            parts: vec![Part::Text(task.to_string())],
            parts_metadata: None,
        };

        // Call run_file_agent
        match run_file_agent(&orchestrator, tool_response, context.clone(), task).await {
            Ok(response) => {
                // Convert ToolResponse back to Vec<Part>
                Ok(response
                    .parts
                    .into_iter()
                    .filter_map(|part| match part {
                        distri_types::Part::Artifact(artifact) => Some(Part::Artifact(artifact)),
                        _ => None,
                    })
                    .collect())
            }
            Err(e) => Err(AgentError::ToolExecution(format!(
                "Artifact analysis failed: {}",
                e
            ))),
        }
    }
}

/// Execute code in a sandboxed browsr shell session.
/// Supports JavaScript, Python, and Bash.
#[derive(Debug)]
pub struct DistriExecuteCodeTool;

#[async_trait::async_trait]
impl Tool for DistriExecuteCodeTool {
    fn get_name(&self) -> String {
        "distri_execute_code".to_string()
    }

    fn get_description(&self) -> String {
        "Execute code in a sandboxed shell session. Supports JavaScript (Node.js), Python, and Bash. Code runs in an isolated container and returns stdout, stderr, exit code, and duration.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "The code to execute"
                },
                "language": {
                    "type": "string",
                    "enum": ["javascript", "python", "bash"],
                    "description": "Programming language (auto-detected if not specified)"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "DistriExecuteCodeTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DistriExecuteCodeTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let code = tool_call
            .input
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'code' parameter".to_string()))?;

        let language = tool_call
            .input
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from);

        match crate::tools::code::execute_code_with_tools(code, language, context).await {
            Ok((result, _observations, _)) => Ok(vec![Part::Data(result)]),
            Err(e) => Err(AgentError::ToolExecution(format!(
                "Code execution failed: {}",
                e
            ))),
        }
    }
}
