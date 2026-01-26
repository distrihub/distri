use distri_types::filesystem::FileSystemOps;
use distri_types::{MessageRole, Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

use crate::agent::todos::TodosTool;
use crate::tools::browser::{
    BrowserStepTool, DistriBrowserSharedTool, DistriScrapeSharedTool, SearchTool,
};
use crate::{
    agent::{file::run_file_agent, ExecutorContext},
    tools::{emit_final, state::AgentExecutorState, ExecutorContextTool},
    types::{Message, ToolCall, ToolResponse},
    AgentError,
};

pub fn get_builtin_tools(
    workspace_filesystem: Arc<distri_filesystem::FileSystem>,
    session_filesystem: Arc<distri_filesystem::FileSystem>,
    include_filesystem_tools: bool,
) -> Vec<Arc<dyn Tool>> {
    let mut tools = vec![
        Arc::new(TransferToAgentTool) as Arc<dyn Tool>,
        Arc::new(FinalTool) as Arc<dyn Tool>,
        Arc::new(DistriScrapeSharedTool) as Arc<dyn Tool>,
        Arc::new(DistriBrowserSharedTool) as Arc<dyn Tool>,
        Arc::new(BrowserStepTool) as Arc<dyn Tool>,
        Arc::new(SearchTool) as Arc<dyn Tool>,
        Arc::new(TodosTool) as Arc<dyn Tool>,
    ];

    #[cfg(feature = "code")]
    {
        tools.push(Arc::new(DistriExecuteCodeTool) as Arc<dyn Tool>);
    }

    if include_filesystem_tools {
        tools.push(Arc::new(ArtifactTool) as Arc<dyn Tool>);
        // File operations should target the workspace filesystem; artifact tools use the session filesystem.
        tools.extend(distri_filesystem::create_core_filesystem_tools(
            workspace_filesystem.clone() as Arc<dyn FileSystemOps>,
        ));

        // Add artifact tools
        tools.extend(distri_filesystem::create_artifact_tools(
            session_filesystem.clone() as Arc<dyn FileSystemOps>,
        ));
    }

    tools
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

/// Implementation of the transfer_to_agent built-in tool
#[derive(Debug)]
pub struct TransferToAgentTool;

#[async_trait::async_trait]
impl Tool for TransferToAgentTool {
    fn get_name(&self) -> String {
        "transfer_to_agent".to_string()
    }
    fn get_description(&self) -> String {
        "Transfer control to another agent to continue the workflow".to_string()
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
                        "reason": {
                            "type": "string",
                            "description": "Optional reason for the transfer"
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

        // Check if target agent exists
        let orchestrator = context.get_orchestrator()?;
        if let Some(_target_agent) = orchestrator.get_agent(target_agent).await {
            // Send handover message through coordinator
            if let Err(e) = orchestrator
                .coordinator_tx
                .send(crate::agent::CoordinatorMessage::HandoverAgent {
                    from_agent: context.agent_id.clone(),
                    to_agent: target_agent.to_string(),
                    reason: reason.clone(),
                    context: context.clone(),
                })
                .await
            {
                tracing::error!("Failed to send handover message: {}", e);
                return Err(AgentError::ToolExecution(format!(
                    "Failed to send handover message: {}",
                    e
                )));
            }

            tracing::info!(
                "Agent handover requested from {} to {}",
                context.agent_id,
                target_agent
            );
            let result = json!({
                "status": "success",
                "message": format!(
                    "Transfer initiated to agent '{}'. Reason: {}",
                    target_agent,
                    reason.unwrap_or_else(|| "No reason provided".to_string())
                )
            });
            // Return as data part
            return Ok(vec![Part::Data(result)]);
        }
        // Agent not found
        Err(AgentError::ToolExecution(format!(
            "Target agent '{}' not found",
            target_agent
        )))
    }
}

#[cfg(feature = "code")]
#[derive(Debug)]
pub struct DistriExecuteCodeTool;

#[cfg(feature = "code")]
#[async_trait::async_trait]
impl Tool for DistriExecuteCodeTool {
    fn get_name(&self) -> String {
        "distri_execute_code".to_string()
    }

    fn is_sync(&self) -> bool {
        false
    }

    fn needs_executor_context(&self) -> bool {
        true // This tool needs ExecutorContext
    }

    fn get_description(&self) -> String {
        "Execute TypeScript/JavaScript code with access to console_log and final_answer tools"
            .to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thought": {
                    "type": "string",
                    "description": "Reasoning about what the code will do"
                },
                "code": {
                    "type": "string",
                    "description": "TypeScript/JavaScript code to execute"
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
        // This should never be called since needs_executor_context() returns true
        Err(anyhow::anyhow!(
            "DistriExecuteCodeTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[cfg(feature = "code")]
#[async_trait::async_trait]
impl ExecutorContextTool for DistriExecuteCodeTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: crate::types::ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, crate::AgentError> {
        // Parse the input to extract code
        let input = tool_call.input.clone();
        let code = input
            .get("code")
            .and_then(|c| c.as_str())
            .ok_or_else(|| crate::AgentError::ToolExecution("Missing 'code' field".to_string()))?;

        // Execute the code with centralized tool setup
        match crate::tools::execute_code_with_tools(code, context).await {
            Ok((result, observations, _)) => {
                let value = json!({
                    "result": result,
                    "observations": observations
                });
                Ok(vec![Part::Data(value)])
            }
            Err(e) => {
                // On failure, report error in result
                let value = json!({
                    "result": format!("Code execution failed: {}", e),
                });
                Ok(vec![Part::Data(value)])
            }
        }
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
        };

        // Use regular execute instead of execute_stream to avoid circular event handling
        let child_context_arc = Arc::new(child_context);
        let child_context_clone = child_context_arc.clone();

        let result = orchestrator
            .execute_stream(&self.agent_name, message, child_context_arc, None)
            .await;

        // Return result from child agent
        let res = match result {
            Ok(invoke_result) => {
                // Get final result from the child context if available
                let final_result = child_context_clone.get_final_result().await;
                let response_text = final_result
                    .or(invoke_result.content.map(|c| Value::String(c)))
                    .unwrap_or_else(|| "Child agent completed without response".to_string().into());

                Ok(vec![Part::Data(response_text)])
            }
            Err(e) => Ok(vec![Part::Data(Value::String(e.to_string()))]),
        };
        res
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
