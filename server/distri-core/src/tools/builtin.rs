use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::todos::TodosTool;
use crate::tools::browser::{
    BrowserStepTool, CrawlTool, DistriBrowserSharedTool, DistriScrapeSharedTool, SearchTool,
};
use crate::tools::save_artifact::SaveArtifactTool;
use crate::tools::shell::{ExecuteShellTool, StartShellTool, StopShellTool};
use crate::{
    agent::{file::run_file_agent, ExecutorContext},
    tools::{emit_final, state::AgentExecutorState, ExecutorContextTool},
    types::{ToolCall, ToolResponse},
    AgentError,
};

/// Returns the set of builtin tools available to all agents.
/// Filesystem tools are no longer included as builtins — they should be
/// provided as external tools by the client or accessed via shell commands.
pub fn get_builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
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
        Arc::new(SaveArtifactTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::supervisor::GetTaskTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::supervisor::WaitTaskTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::supervisor::CancelTaskTool) as Arc<dyn Tool>,
        Arc::new(crate::tools::supervisor::ListMyTasksTool) as Arc<dyn Tool>,
    ]
}

/// Typed representation of the `final` tool's input.
/// The LLM may pass the result as a bare string or wrapped as `{"input": ...}`.
#[derive(Debug, serde::Deserialize)]
struct FinalInput {
    input: serde_json::Value,
}

/// Final tool for code execution mode with state management
#[derive(Debug)]
pub struct FinalTool;

impl FinalTool {
    /// Extract the final result value from the raw JSON input of a `final` tool call.
    ///
    /// Handles two forms:
    /// - Wrapped: `{"input": <value>}` → returns the inner value
    /// - Direct: any other JSON value → returned as-is
    ///
    /// Returns `Err(String)` if the input looks like a wrapped object but cannot
    /// be deserialized into `FinalInput`.
    pub fn extract_result(raw: &serde_json::Value) -> Result<serde_json::Value, String> {
        match raw {
            serde_json::Value::Object(_) => serde_json::from_value::<FinalInput>(raw.clone())
                .map(|fi| fi.input)
                .map_err(|e| format!("final tool input is malformed: {e}")),
            other => Ok(other.clone()),
        }
    }
}

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
