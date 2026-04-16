use distri_types::{MessageRole, Part, RuntimeMode, Tool, ToolContext};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

use crate::agent::todos::TodosTool;
use crate::tools::browser::{
    BrowserStepTool, CrawlTool, DistriBrowserSharedTool, DistriScrapeSharedTool, SearchTool,
};
use crate::tools::chart::RenderChartTool;
use crate::tools::shell::{ExecuteShellTool, StartShellTool, StopShellTool};
use crate::{
    agent::{
        context::{ForkOptions, ForkType},
        file::run_file_agent,
        ExecutorContext,
    },
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
        Arc::new(RenderChartTool) as Arc<dyn Tool>,
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
                    .or(invoke_result.content.map(Value::String))
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
        let use_remote = orchestrator.background_runner.is_some() && {
            let agent_def = orchestrator.stores.agent_store.get(&self.agent_name).await;
            matches!(
                agent_def,
                Some(distri_types::configuration::AgentConfig::StandardAgent(ref def)) if def.remote
            )
        };

        if use_remote {
            // DeepAgent path: spawn in sandbox, subscribe to events, relay to parent
            let runner = orchestrator.background_runner.as_ref().unwrap();
            let broadcaster = orchestrator.broadcaster();
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
                    Some(context.thread_id.clone()),
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to spawn deepagent: {}", e))
                })?;

            // Subscribe to sub-task events, relay to parent event channel
            let mut stream = broadcaster.subscribe(&sub_task_id).await.map_err(|e| {
                AgentError::ToolExecution(format!("Failed to subscribe to deepagent events: {}", e))
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

            Ok(vec![Part::Data(final_result.unwrap_or_else(|| {
                Value::String("Deepagent completed".to_string())
            }))])
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
                        .or(invoke_result.content.map(Value::String))
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

// ── UniversalAgentTool ──────────────────────────────────────────────────────

/// Built-in agent names that are always available regardless of sub_agents config.
pub(crate) const ALWAYS_AVAILABLE_BUILTINS: &[&str] = &[
    "_system/plan",
    "_system/coder",
    "distri_runner",
    "distri_browser_runner",
];

/// Built-in agents that are only available when explicitly listed in sub_agents.
#[allow(dead_code)] // Used by tests
pub(crate) const OPT_IN_BUILTINS: &[&str] = &["_system/explore"];

/// Normalize a short builtin name to its canonical `_system/` prefixed form.
/// e.g. "plan" → "_system/plan", "coder" → "_system/coder".
/// If the name already has a prefix, return it unchanged.
pub(crate) fn normalize_system_agent_name(name: &str) -> String {
    if name.starts_with("_system/") {
        name.to_string()
    } else {
        format!("_system/{}", name)
    }
}

/// Check whether an agent is accessible from the calling agent's context.
///
/// An agent is accessible if:
/// - It is in `ALWAYS_AVAILABLE_BUILTINS` (including short-name matches), OR
/// - The calling agent's `sub_agents` contains `"*"` (wildcard), OR
/// - It is explicitly listed in the calling agent's `sub_agents`
pub(crate) fn is_agent_accessible(agent_name: &str, sub_agents: &[String]) -> bool {
    let canonical = normalize_system_agent_name(agent_name);

    // Always-available builtins are accessible regardless (check both forms)
    if ALWAYS_AVAILABLE_BUILTINS.contains(&agent_name)
        || ALWAYS_AVAILABLE_BUILTINS.contains(&canonical.as_str())
    {
        return true;
    }

    // Wildcard grants access to everything
    if sub_agents.iter().any(|sa| sa == "*") {
        return true;
    }

    // Check if explicitly in sub_agents (exact match or short name match)
    if sub_agents.iter().any(|sa| {
        sa == agent_name || normalize_system_agent_name(sa) == agent_name || sa == &canonical
    }) {
        return true;
    }

    false
}

/// Resolve the logical "code" / "coder" agent to a concrete system agent
/// name based on the caller's runtime. The orchestrator handles any runtime
/// mismatch via `BackgroundRunner` — the resolver itself doesn't need to
/// know about remote dispatch.
///
/// Returns short names (no `_system/` prefix); the caller is expected to
/// run them through `normalize_system_agent_name` + the existing fallback
/// chain so both file-based and baked-in registrations are found.
///
/// Mapping:
/// - `Browser` → `distri_browser_runner`
/// - `Cli` / `Cloud` → `distri_runner` (declares `runtime = ["cli"]`, so a
///   Cloud caller auto-routes via the sandbox launcher).
pub(crate) fn resolve_code_agent(runtime_mode: &RuntimeMode) -> &'static str {
    match runtime_mode {
        RuntimeMode::Browser => "distri_browser_runner",
        RuntimeMode::Cli | RuntimeMode::Cloud => "distri_runner",
    }
}

/// Input struct for the `call_agent` tool.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct CallAgentInput {
    /// Name of the agent to call. If omitted, an ad-hoc agent is created from system_prompt.
    #[serde(default)]
    pub(crate) agent: Option<String>,
    /// The task/prompt to send to the agent.
    pub(crate) prompt: String,
    /// System prompt for ad-hoc agent creation. When set without `agent`, creates a temporary agent.
    #[serde(default)]
    pub(crate) system_prompt: Option<String>,
    /// Tool names to give the ad-hoc agent (only used with system_prompt).
    #[serde(default)]
    pub(crate) tools: Option<Vec<String>>,
    /// Model override for the ad-hoc agent.
    #[serde(default)]
    pub(crate) model: Option<String>,
    /// When true, fork the current context (copy history) instead of creating a clean sub-task.
    #[serde(default)]
    pub(crate) fork: bool,
    /// Description for the ad-hoc agent.
    #[serde(default)]
    pub(crate) description: Option<String>,
    /// When true, launch the agent in the background and return immediately.
    /// The parent will be notified when the child completes.
    #[serde(default)]
    pub(crate) run_in_background: bool,
    /// Optional name for the background agent (used by SendMessage for routing).
    #[serde(default)]
    pub(crate) name: Option<String>,
}

/// Universal agent tool that replaces per-agent `call_<name>` tools with a single
/// `call_agent` tool. Handles resolution, access control, ad-hoc creation, fork,
/// and remote execution internally.
#[derive(Debug)]
pub struct UniversalAgentTool;

#[async_trait::async_trait]
impl Tool for UniversalAgentTool {
    fn get_name(&self) -> String {
        "call_agent".to_string()
    }

    fn get_description(&self) -> String {
        "Call another agent to perform a task. Can target a named agent, or create an ad-hoc agent with a custom system prompt. Supports forking (copying parent history) for continuity.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Name of the agent to call (e.g. 'coder', '_system/plan', 'my_package/my_agent'). Omit to create an ad-hoc agent using system_prompt."
                },
                "prompt": {
                    "type": "string",
                    "description": "The task or instruction to send to the agent."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "System prompt for creating an ad-hoc agent. Only used when 'agent' is not specified."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names to give the ad-hoc agent (only with system_prompt)."
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the agent (only with system_prompt)."
                },
                "fork": {
                    "type": "boolean",
                    "description": "When true, fork the current context so the child agent sees the parent's message history.",
                    "default": false
                },
                "description": {
                    "type": "string",
                    "description": "Description for the ad-hoc agent (only with system_prompt)."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, launch the agent in the background and return immediately. You will be notified when it completes.",
                    "default": false
                },
                "name": {
                    "type": "string",
                    "description": "Optional name for the background agent (used for inter-agent messaging via send_message)."
                }
            },
            "required": ["prompt"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "UniversalAgentTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for UniversalAgentTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Parse input
        let input: CallAgentInput = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid call_agent input: {}", e)))?;

        let orchestrator = context.get_orchestrator()?;

        // ── Resolve agent name ────────────────────────────────────────────
        let agent_name = if let Some(ref name) = input.agent {
            // Named agent: normalize builtin short names
            let normalized = if !name.contains('/') && !name.starts_with("_system/") {
                // Could be a short builtin name like "plan" or "coder"
                let candidate = normalize_system_agent_name(name);
                // Check if it exists as a builtin, otherwise keep original
                if orchestrator.get_agent(&candidate).await.is_some() {
                    candidate
                } else {
                    name.to_string()
                }
            } else {
                name.to_string()
            };

            // Special handling: resolve the logical "code" / "coder" agent
            // to a concrete system agent based on the caller's runtime.
            // Cli/Cloud → distri_runner (Cloud auto-routes via sandbox because
            // distri_runner declares `runtime = ["cli"]`); Browser → distri_browser_runner.
            let is_code_alias = matches!(
                normalized.as_str(),
                "_system/coder" | "coder" | "_system/code" | "code"
            );
            if is_code_alias {
                let short = resolve_code_agent(&context.runtime_mode);
                // Run the resolver result through the same normalize+fallback
                // chain so it works whether the agent is baked-in (`_system/<name>`)
                // or file-loaded (`<name>` from agents/ dir).
                let candidate = normalize_system_agent_name(short);
                if orchestrator.get_agent(&candidate).await.is_some() {
                    candidate
                } else {
                    short.to_string()
                }
            } else {
                normalized
            }
        } else if input.system_prompt.is_some() {
            // Ad-hoc mode: create a temporary agent from system_prompt
            let adhoc_name = format!("_adhoc/{}", uuid::Uuid::new_v4());
            let system_prompt = input.system_prompt.as_deref().unwrap_or_default();

            let mut definition = distri_types::StandardDefinition {
                name: adhoc_name.clone(),
                description: input
                    .description
                    .clone()
                    .unwrap_or_else(|| "Ad-hoc agent".to_string()),
                instructions: system_prompt.to_string(),
                ..Default::default()
            };

            // Set model if specified
            if let Some(ref model) = input.model {
                definition.model_settings = Some(distri_types::ModelSettings::new(model.clone()));
            }

            // Set tools if specified
            if let Some(ref tool_names) = input.tools {
                definition.tools = Some(distri_types::ToolsConfig {
                    builtin: tool_names.clone(),
                    ..Default::default()
                });
            }

            // Register the ad-hoc agent
            let agent_config = distri_types::configuration::AgentConfig::StandardAgent(definition);
            orchestrator
                .stores
                .agent_store
                .register(agent_config)
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to register ad-hoc agent: {}", e))
                })?;

            adhoc_name
        } else {
            return Err(AgentError::ToolExecution(
                "call_agent requires either 'agent' name or 'system_prompt' for ad-hoc creation"
                    .to_string(),
            ));
        };

        // ── Access control ────────────────────────────────────────────────
        // Get the calling agent's sub_agents list
        let calling_agent_sub_agents = orchestrator
            .get_agent(&context.agent_id)
            .await
            .and_then(|cfg| match cfg {
                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                    Some(def.sub_agents)
                }
                _ => None,
            })
            .unwrap_or_default();

        if !is_agent_accessible(&agent_name, &calling_agent_sub_agents) {
            return Err(AgentError::ToolExecution(format!(
                "Agent '{}' is not accessible from '{}'. Add it to sub_agents or use an always-available builtin.",
                agent_name, context.agent_id
            )));
        }

        // Verify agent exists
        if orchestrator.get_agent(&agent_name).await.is_none() {
            return Err(AgentError::ToolExecution(format!(
                "Agent '{}' not found",
                agent_name
            )));
        }

        // ── Cloud coder: sandbox → remote, otherwise → coder_lite ─────────
        let agent_name = if agent_name == "_system/coder"
            && context.runtime_mode == RuntimeMode::Cloud
        {
            let has_sandbox = orchestrator.background_runner.is_some();
            if has_sandbox {
                // Sandbox available: run _system/coder remotely (distri-cli in browsr container)
                agent_name
            } else {
                // No sandbox: fall back to coder_lite which has browsr shell tools built-in
                tracing::info!("Cloud mode without sandbox: falling back to _system/coder_lite");
                "_system/coder_lite".to_string()
            }
        } else {
            agent_name
        };

        // ── Fire-and-forget path: run_in_background via coordinator + broadcaster ──
        if input.run_in_background {
            let coordinator = orchestrator.coordinator();
            let child_task_id = uuid::Uuid::new_v4().to_string();
            let child_message = Message::user(input.prompt.clone(), None);

            // Register task with coordinator
            let cancel_signal = coordinator
                .register_task(&child_task_id, &context.thread_id, input.name.as_deref())
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to register background task: {}", e))
                })?;

            // Take mailbox for inter-agent messaging
            let mailbox = coordinator.take_mailbox(&child_task_id).await.ok();

            // Create child execution context
            let child_context = context.new_task(&agent_name).await;
            let (event_tx, mut event_rx) =
                tokio::sync::mpsc::channel::<distri_types::AgentEvent>(100);
            let mut child_ctx = child_context.clone_with_tx(event_tx);
            child_ctx.task_id = child_task_id.clone();
            child_ctx.cancellation_signal = Some(cancel_signal);
            if let Some(mb) = mailbox {
                child_ctx.mailbox = Some(Arc::new(tokio::sync::Mutex::new(mb)));
            }
            child_ctx.parent_task_id = Some(context.task_id.clone());
            let child_ctx_arc = Arc::new(child_ctx);

            // Spawn event relay: child events → broadcaster
            let runtime_for_relay = orchestrator.runtime.clone();
            let task_id_for_relay = child_task_id.clone();
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let is_terminal = matches!(
                        &event.event,
                        distri_types::AgentEventType::RunFinished { .. }
                            | distri_types::AgentEventType::RunError { .. }
                    );
                    let task_id = event.task_id.clone();
                    let _ = runtime_for_relay
                        .broadcaster()
                        .publish(&task_id, event)
                        .await;
                    if is_terminal {
                        let _ = runtime_for_relay
                            .coordinator()
                            .complete_task(&task_id_for_relay)
                            .await;
                        break;
                    }
                }
            });

            // Spawn the actual agent execution in background
            let orchestrator_clone = orchestrator.clone();
            let agent_name_clone = agent_name.clone();
            let user_id = context.user_id.clone();
            let workspace_id = context
                .workspace_id
                .as_ref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            tokio::spawn(distri_auth::context::with_user_and_workspace(
                user_id,
                workspace_id,
                async move {
                    let result = orchestrator_clone
                        .execute_stream(
                            &agent_name_clone,
                            child_message,
                            child_ctx_arc.clone(),
                            None,
                        )
                        .await;
                    if let Ok(invoke_result) = result {
                        if let Some(content) = &invoke_result.content {
                            let final_msg = distri_types::Message::assistant(content.clone(), None);
                            child_ctx_arc.save_message(&final_msg).await;
                        }
                    }
                },
            ));

            // Return immediately — parent continues executing
            return Ok(vec![Part::Data(json!({
                "status": "async_launched",
                "task_id": child_task_id,
                "agent": agent_name,
                "message": "Agent launched in background. You will be notified when it completes."
            }))]);
        }

        // ── Check if remote execution is needed ───────────────────────────
        // In Cloud mode, _system/coder always runs remotely via SandboxLauncher.
        // (coder_lite runs in-process with browsr shell tools.)
        let use_remote = orchestrator.background_runner.is_some()
            && agent_name == "_system/coder"
            && context.runtime_mode == RuntimeMode::Cloud;

        if use_remote {
            // ── Remote/DeepAgent path ─────────────────────────────────────
            let runner = orchestrator.background_runner.as_ref().unwrap();
            let broadcaster = orchestrator.broadcaster();
            let sub_task_id = uuid::Uuid::new_v4().to_string();
            let env_id = format!(
                "{}-{}",
                context.workspace_id.as_deref().unwrap_or("default"),
                agent_name
            );

            tracing::info!(
                "UniversalAgentTool: remote dispatch {} (task_id={})",
                agent_name,
                sub_task_id
            );

            runner
                .spawn(
                    sub_task_id.clone(),
                    agent_name.clone(),
                    input.prompt.clone(),
                    context.user_id.clone(),
                    context.workspace_id.clone(),
                    Some(env_id),
                    None,
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!("Failed to spawn remote agent: {}", e))
                })?;

            let mut stream = broadcaster.subscribe(&sub_task_id).await.map_err(|e| {
                AgentError::ToolExecution(format!(
                    "Failed to subscribe to remote agent events: {}",
                    e
                ))
            })?;

            let mut final_result: Option<Value> = None;

            use futures_util::StreamExt;
            while let Some(event) = stream.next().await {
                context.emit(event.event.clone()).await;

                match &event.event {
                    distri_types::AgentEventType::RunFinished { .. } => {
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
                        final_result =
                            Some(Value::String(format!("Remote agent error: {}", message)));
                        break;
                    }
                    _ => {}
                }
            }

            Ok(vec![Part::Data(final_result.unwrap_or_else(|| {
                Value::String("Remote agent completed".to_string())
            }))])
        } else if input.fork {
            // ── Fork path: copy parent history into child ─────────────────
            let forked_context = context
                .fork(ForkOptions {
                    fork_type: ForkType::NewTask,
                    copy_history_limit: None,
                })
                .await;
            let mut forked_context = forked_context;
            forked_context.agent_id = agent_name.clone();

            // Copy parent's message history to the forked task
            let parent_history = context
                .get_current_task_message_history()
                .await
                .unwrap_or_default();

            for msg in &parent_history {
                let store_msg = distri_types::Message {
                    id: msg.id.clone(),
                    name: msg.name.clone(),
                    parts: msg.parts.clone(),
                    role: msg.role.clone(),
                    created_at: msg.created_at,
                    agent_id: msg.agent_id.clone(),
                    parts_metadata: msg.parts_metadata.clone(),
                };
                let _ = orchestrator
                    .stores
                    .task_store
                    .add_message_to_task(&forked_context.task_id, &store_msg)
                    .await;
            }

            // Add the fork directive message
            let fork_message = Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![Part::Text(format!(
                    "[Forked from parent agent '{}'. Continue with the following task:]\n\n{}",
                    context.agent_id, input.prompt
                ))],
                role: MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let forked_context_arc = Arc::new(forked_context);
            let forked_context_clone = forked_context_arc.clone();

            let result = orchestrator
                .execute_stream(&agent_name, fork_message, forked_context_arc, None)
                .await;

            match result {
                Ok(invoke_result) => {
                    let final_result = forked_context_clone.get_final_result().await;
                    let response_text = final_result
                        .or(invoke_result.content.map(Value::String))
                        .unwrap_or_else(|| {
                            Value::String("Forked agent completed without response".to_string())
                        });
                    Ok(vec![Part::Data(response_text)])
                }
                Err(e) => Ok(vec![Part::Data(Value::String(e.to_string()))]),
            }
        } else {
            // ── Standard in-process path ──────────────────────────────────
            let child_context = context.new_task(&agent_name).await;

            let message = Message {
                id: uuid::Uuid::new_v4().to_string(),
                name: None,
                parts: vec![Part::Text(input.prompt.clone())],
                role: MessageRole::User,
                created_at: chrono::Utc::now().timestamp_millis(),
                agent_id: None,
                parts_metadata: None,
            };

            let child_context_arc = Arc::new(child_context);
            let child_context_clone = child_context_arc.clone();

            let result = orchestrator
                .execute_stream(&agent_name, message, child_context_arc, None)
                .await;

            match result {
                Ok(invoke_result) => {
                    let final_result = child_context_clone.get_final_result().await;
                    let response_text = final_result
                        .or(invoke_result.content.map(Value::String))
                        .unwrap_or_else(|| {
                            Value::String("Child agent completed without response".to_string())
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
