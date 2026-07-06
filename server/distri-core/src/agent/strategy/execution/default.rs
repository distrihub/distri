use crate::{
    agent::{
        strategy::execution::{ExecutionResult, ExecutionStrategy},
        AgentEventType, ExecutorContext, InvokeResult,
    },
    tools::Tool,
    AgentError,
};
use distri_types::{
    Action, ExecutionStatus, Part, PlanStep, StandardDefinition, ToolResponse, ToolResultWithSkip,
    DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS,
};
use std::{sync::Arc, time::Duration};

/// Unified AgentExecutor that combines functionality from all execution strategies
pub struct AgentExecutor {
    tools: Vec<Arc<dyn Tool>>,
    agent_definition: Option<StandardDefinition>,
    external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
}

impl AgentExecutor {
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        agent_definition: Option<StandardDefinition>,
        external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    ) -> Self {
        Self {
            tools,
            agent_definition,
            external_tool_calls_store,
        }
    }

    /// Unified tool call handler that executes tool calls and handles responses
    /// Returns (tool_results, code_state) to maintain state across executions
    pub async fn handle_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<ExecutorContext>,
        step_id: &str,
        step: &PlanStep,
    ) -> Result<ToolResultResponse, AgentError> {
        if tool_calls.is_empty() {
            return Err(AgentError::Validation("No tool calls provided".to_string()));
        }

        // Get all available tools from context (including MCP tools)
        let enhanced_tools = context.get_tools().await;

        // Get external tool timeout from agent definition strategy
        let external_tool_timeout_secs = self
            .agent_definition
            .as_ref()
            .and_then(|def| def.strategy.as_ref())
            .map(|s| s.get_external_tool_timeout_secs())
            .unwrap_or(DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS);

        // NOTE: the ToolCalls event is emitted inside
        // `execute_tool_calls_with_timeout` AFTER all external tool calls are
        // pre-registered — this closes the race where a client receiving the
        // event could call `complete_tool` before the server has a pending
        // receiver set up, which used to surface as "complete_tool timed out
        // after retries — server never registered the pending call".

        // Execute tool calls with configurable timeout
        let tool_results = execute_tool_calls_with_timeout(
            self.external_tool_calls_store.clone(),
            tool_calls,
            context.clone(),
            &enhanced_tools,
            step_id,
            external_tool_timeout_secs,
        )
        .await?;

        // Handle tool responses
        self.handle_tool_responses(&tool_results, context.clone(), step_id, step)
            .await
    }

    async fn handle_tools_action(
        &self,
        response: &InvokeResult,
        context: Arc<ExecutorContext>,
        step_id: &str,
        step: &PlanStep,
    ) -> Result<ExecutionResult, AgentError> {
        // Everything becomes a simple observation now
        let mut parts = vec![];
        if let Some(content) = &response.content {
            parts.push(Part::Text(content.clone()));
        }
        let mut reason = None;
        let mut status = ExecutionStatus::Success;
        if !response.tool_calls.is_empty() {
            for tool_call in &response.tool_calls {
                parts.push(Part::ToolCall(tool_call.clone()));
            }

            let tools_response = self
                .handle_tool_calls(&response.tool_calls, context.clone(), step_id, step)
                .await;

            match tools_response {
                Ok(tools_response) => {
                    // Each ToolResponse already carries its parts (Data,
                    // Image, Text, …) inside `tool_response.parts`. Flat
                    // copies on the assistant side are dead weight: the
                    // formatter would route them onto the assistant
                    // message, where the LLM client silently drops Image
                    // parts (`llm.rs:1226-1262` has no Part::Image branch
                    // for assistant messages). The image_url follow-up
                    // user message is built FROM the tool message's
                    // tool_response.parts (`llm.rs:1290-1304`), so the
                    // Image needs to live there only.
                    for tool_response in tools_response.tool_responses {
                        parts.push(Part::ToolResult(tool_response));
                    }
                    status = if tools_response.input_required {
                        ExecutionStatus::InputRequired
                    } else {
                        ExecutionStatus::Success
                    };
                }
                Err(e) => {
                    status = ExecutionStatus::Failed;
                    reason = Some(e.to_string());
                }
            }

            Ok(ExecutionResult {
                step_id: step_id.to_string(),
                status,
                parts,
                timestamp: chrono::Utc::now().timestamp_millis(),
                reason,
            })
        } else {
            // No tool calls and no parseable code, return as thought
            Ok(ExecutionResult {
                step_id: step_id.to_string(),
                status,
                parts,
                timestamp: chrono::Utc::now().timestamp_millis(),
                reason,
            })
        }
    }
    /// Unified tool response handler that saves results and emits events
    async fn handle_tool_responses(
        &self,
        tool_results: &[ToolResultWithSkip],
        context: Arc<ExecutorContext>,
        step_id: &str,
        _step: &PlanStep,
    ) -> Result<ToolResultResponse, AgentError> {
        if tool_results.is_empty() {
            return Ok(ToolResultResponse::default());
        }

        let mut processed_tool_results: Vec<crate::types::ToolResponse> = Vec::new();

        let mut input_required = false;
        for result in tool_results {
            match result {
                ToolResultWithSkip::ToolResult(tool_result) => {
                    let fields = distri_formatter::extract::extract_fields(tool_result);
                    let content_size = fields.content_size();

                    let processed_response = if content_size
                        > distri_types::tool_result_store::PERSIST_THRESHOLD_BYTES
                    {
                        let file_ref = self
                            .persist_large_result(tool_result, &fields, &context)
                            .await;

                        let file_ref_tuple =
                            file_ref.as_deref().map(|path| (path, content_size / 1024));
                        let formatted = fields.format_plain(
                            distri_types::tool_result_store::PREVIEW_SIZE_BYTES,
                            file_ref_tuple,
                        );

                        crate::types::ToolResponse::from_parts(
                            tool_result.tool_call_id.clone(),
                            tool_result.tool_name.clone(),
                            vec![Part::Text(formatted)],
                        )
                    } else {
                        // Small result: keep raw parts for provider compatibility
                        tool_result.clone()
                    };

                    processed_tool_results.push(processed_response);
                }
                ToolResultWithSkip::Skip { .. } => {
                    input_required = true;
                    // Skip entries are not included in the results
                }
            }
        }

        // Emit tool results event with processed results
        context
            .emit(AgentEventType::ToolResults {
                step_id: step_id.to_string(),
                parent_message_id: context.get_current_message_id().await,
                results: processed_tool_results.clone(),
            })
            .await;

        Ok(ToolResultResponse {
            tool_responses: processed_tool_results,
            input_required,
        })
    }

    /// Persist a large tool result to disk via Write tool or artifact storage fallback.
    /// Returns the file path/reference if persistence succeeded.
    async fn persist_large_result(
        &self,
        tool_result: &crate::types::ToolResponse,
        fields: &distri_formatter::extract::ToolFields,
        context: &Arc<ExecutorContext>,
    ) -> Option<String> {
        let content = fields.large_content();
        if content.is_empty() {
            return None;
        }

        let file_path = format!(".distri/tool-results/{}.txt", tool_result.tool_call_id);

        // Try Write tool first
        let tools = context.get_tools().await;
        let write_tool = crate::agent::tool_lookup::find_tool_by_name(&tools, "Write");

        if let Some(tool) = write_tool {
            let write_call = crate::types::ToolCall {
                tool_call_id: format!("sys_{}", tool_result.tool_call_id),
                tool_name: "Write".to_string(),
                input: serde_json::json!({
                    "file_path": file_path,
                    "content": content,
                }),
            };

            let tool_context = crate::tools::context::to_tool_context(context.as_ref());
            match tool
                .execute(write_call, std::sync::Arc::new(tool_context))
                .await
            {
                Ok(_) => {
                    tracing::debug!(
                        path = %file_path,
                        size = content.len(),
                        "Large tool result persisted via Write"
                    );
                    return Some(file_path);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Write tool failed, falling back to artifact storage"
                    );
                }
            }
        }

        // Fallback: artifact storage
        if let Ok(orchestrator) = context.get_orchestrator() {
            let base_path = distri_filesystem::ArtifactWrapper::task_namespace(
                &context.thread_id,
                &context.task_id,
            );
            if let Ok(wrapper) = orchestrator
                .session_filesystem
                .create_artifact_wrapper(base_path)
                .await
            {
                let filename = format!("{}.txt", tool_result.tool_call_id);
                match wrapper.save_artifact(&filename, &content).await {
                    Ok(()) => {
                        tracing::debug!(
                            filename = %filename,
                            "Large tool result persisted to artifacts"
                        );
                        return Some(format!("artifact:{}", filename));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Artifact storage also failed");
                    }
                }
            }
        }

        tracing::warn!("Could not persist large tool result");
        None
    }
}

impl std::fmt::Debug for AgentExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentExecutor")
            .field("tools", &format!("{} tools", self.tools.len()))
            .finish()
    }
}

#[async_trait::async_trait]
impl ExecutionStrategy for AgentExecutor {
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError> {
        tracing::debug!("Executing step: {:?}", step);

        let action = &step.action;

        match action {
            Action::ToolCalls { tool_calls } => {
                self.handle_tools_action(
                    &InvokeResult {
                        content: None,
                        tool_calls: tool_calls.to_vec(),
                    },
                    context.clone(),
                    &step.id,
                    step,
                )
                .await
            }
            Action::Code { code, .. } => {
                match crate::tools::execute_code_with_tools(code, None, context.clone()).await {
                    Ok((_, observations, _)) => Ok(ExecutionResult {
                        step_id: step.id.clone(),
                        status: ExecutionStatus::Success,
                        parts: vec![Part::Text(observations.join("\n\n"))],
                        timestamp: chrono::Utc::now().timestamp_millis(),
                        reason: None,
                    }),
                    Err(e) => {
                        tracing::warn!("Code execution failed: {}", e);
                        Ok(ExecutionResult {
                            step_id: step.id.clone(),
                            status: ExecutionStatus::Failed,
                            parts: vec![],
                            timestamp: chrono::Utc::now().timestamp_millis(),
                            reason: Some(format!("Code execution error: {}", e)),
                        })
                    }
                }
            }
        }
    }

    async fn execute_step_stream(
        &self,
        step: &PlanStep,
        context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError> {
        // For code steps, use non-streaming version
        self.execute_step(step, context).await
    }

    async fn should_continue(
        &self,
        _plan: &[PlanStep],
        _current_index: usize,
        context: Arc<ExecutorContext>,
    ) -> bool {
        if context.get_final_result().await.is_some() {
            return false;
        }
        if !matches!(
            context.get_status().await,
            Some(crate::types::TaskStatus::Running)
        ) {
            return false;
        }
        // A tool can declaratively END the agent's turn by returning
        // `should_continue: false` in a `data` part of its result. This is how an
        // interactive / human-in-the-loop "checkpoint" tool (e.g. a UI tool that
        // asks the user a question and waits) stops the agent from chaining into
        // the next tool: after the user's answer comes back, the turn ends here and
        // the human's next message (or a "continue" action) starts a fresh turn.
        let history = context.get_execution_history().await;
        if let Some(last) = history.last() {
            if parts_request_turn_end(&last.parts) {
                return false;
            }
        }
        true
    }
}

/// True when the last step's parts request the agent's turn to END — i.e. a
/// checkpoint tool returned `should_continue: false` in a `Part::Data`.
///
/// Since #119 tool results are wrapped as `Part::ToolResult(tool_response)`
/// rather than flattened to top-level `Part::Data`, so we must inspect BOTH the
/// top-level parts AND each tool result's own nested parts. Missing the nested
/// case silently broke the human-in-the-loop `confirm_plan` / `ask_questions`
/// checkpoints (the turn never ended, so the flow ran past the checkpoint).
pub(crate) fn parts_request_turn_end(parts: &[Part]) -> bool {
    fn says_stop(v: &serde_json::Value) -> bool {
        v.get("should_continue").and_then(|b| b.as_bool()) == Some(false)
    }
    for part in parts {
        match part {
            Part::Data(v) if says_stop(v) => return true,
            Part::ToolResult(tr) => {
                for inner in &tr.parts {
                    if let Part::Data(v) = inner {
                        if says_stop(v) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod should_continue_tests {
    use super::parts_request_turn_end;
    use crate::types::ToolResponse;
    use distri_types::Part;
    use serde_json::json;

    fn tool_result(data: serde_json::Value) -> Part {
        Part::ToolResult(ToolResponse::from_parts(
            "call_1".to_string(),
            "confirm_plan".to_string(),
            vec![Part::Data(data)],
        ))
    }

    #[test]
    fn stop_when_nested_in_tool_result() {
        // The #119 regression case: `should_continue: false` lives inside a
        // wrapped Part::ToolResult (a checkpoint tool's answer). Must stop.
        let parts = vec![
            Part::ToolCall(crate::types::ToolCall {
                tool_call_id: "call_1".into(),
                tool_name: "confirm_plan".into(),
                input: json!({}),
            }),
            tool_result(json!({ "approved": true, "ai_decide": true, "should_continue": false })),
        ];
        assert!(parts_request_turn_end(&parts));
    }

    #[test]
    fn stop_when_top_level_data() {
        // Legacy flat shape still honored.
        let parts = vec![Part::Data(json!({ "should_continue": false }))];
        assert!(parts_request_turn_end(&parts));
    }

    #[test]
    fn continue_when_no_flag() {
        let parts = vec![tool_result(json!({ "approved": true }))];
        assert!(!parts_request_turn_end(&parts));
    }

    #[test]
    fn continue_when_flag_true() {
        let parts = vec![tool_result(json!({ "should_continue": true }))];
        assert!(!parts_request_turn_end(&parts));
    }

    #[test]
    fn continue_on_plain_tool_result() {
        // A normal tool result (no checkpoint) must never end the turn.
        let parts = vec![tool_result(json!({ "ok": true, "id": "lesson_1" }))];
        assert!(!parts_request_turn_end(&parts));
    }
}

pub async fn execute_tool_calls(
    external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    tool_calls: &[crate::types::ToolCall],
    context: Arc<ExecutorContext>,
    tools: &[Arc<dyn Tool>],
    step_id: &str,
) -> Result<Vec<ToolResultWithSkip>, AgentError> {
    execute_tool_calls_with_timeout(
        external_tool_calls_store,
        tool_calls,
        context,
        tools,
        step_id,
        DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS,
    )
    .await
}

pub async fn execute_tool_calls_with_timeout(
    external_tool_calls_store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    tool_calls: &[crate::types::ToolCall],
    context: Arc<ExecutorContext>,
    tools: &[Arc<dyn Tool>],
    step_id: &str,
    external_tool_timeout_secs: u64,
) -> Result<Vec<ToolResultWithSkip>, AgentError> {
    // separate internal tool calls only

    tracing::debug!("tool_calls: {tool_calls:?}");
    tracing::debug!("Available tools: {tools:?}");
    let tool_tuples = tool_calls
        .iter()
        .map(|tool_call| {
            let tool = tools
                .iter()
                .find(|t| t.get_name() == tool_call.tool_name)
                .map(|tool| (tool, tool_call.clone()));
            match tool {
                Some((tool, tool_call)) => Ok((tool, tool_call)),
                None => Err(AgentError::ToolExecution(format!(
                    "Tool '{}' not found",
                    tool_call.tool_name
                ))),
            }
        })
        .collect::<Result<Vec<_>, AgentError>>()?;

    // Pre-register every external tool call BEFORE emitting the ToolCalls
    // event. Previously the server emitted ToolCalls, then entered the
    // per-tool `handle_external_tool_inline` which called
    // `register_external_tool_call`. If the client finished executing the
    // tool and hit `/complete-tool` before the server reached that
    // registration, the server returned "No pending…" and the client
    // retried with exponential backoff — but after 10 retries (~11s total)
    // the client gave up with "complete_tool timed out after retries —
    // server never registered the pending call". Registering before
    // emitting closes the race by construction.
    use std::collections::HashMap;
    let mut pending_receivers: HashMap<
        String,
        tokio::sync::oneshot::Receiver<distri_types::ToolResponse>,
    > = HashMap::new();
    for (tool, tool_call) in &tool_tuples {
        if tool.is_external() {
            match external_tool_calls_store
                .register_external_tool_call(&tool_call.tool_call_id)
                .await
            {
                Ok(rx) => {
                    tracing::info!(
                        target: "ext_tool.register",
                        task_id = %context.task_id,
                        agent_id = %context.agent_id,
                        parent_task_id = %context.parent_task_id.as_deref().unwrap_or("-"),
                        tool_call_id = %tool_call.tool_call_id,
                        tool_name = %tool_call.tool_name,
                        "registered pending external tool call (waiting for browser)"
                    );
                    pending_receivers.insert(tool_call.tool_call_id.clone(), rx);
                }
                Err(e) => {
                    tracing::error!(
                        tool_call_id = %tool_call.tool_call_id,
                        tool = %tool_call.tool_name,
                        "failed to pre-register external tool call: {}",
                        e
                    );
                    // Fall through: handle_external_tool_inline will attempt
                    // a late registration and return a proper error if it
                    // still fails — preserving the old error shape rather
                    // than silently dropping the call.
                }
            }
        }
    }

    // Now it is safe to tell the client about these tool calls — the
    // receivers are in place so `complete_tool` cannot race us.
    context
        .emit(AgentEventType::ToolCalls {
            step_id: step_id.to_string(),
            parent_message_id: context.get_current_message_id().await,
            tool_calls: tool_calls.to_vec(),
        })
        .await;

    let step_id = step_id.to_string();
    let timeout = Duration::from_secs(external_tool_timeout_secs);

    // Wrap in Arc<Mutex> so each spawned future can steal its own receiver.
    let pending_receivers = Arc::new(tokio::sync::Mutex::new(pending_receivers));

    // Concurrency policy for this batch: if every tool is concurrency-safe we
    // run them all in parallel; if any tool mutates shared state we serialize
    // the whole batch (permits = 1) so writes can't race. Result mapping is by
    // `tool_call_id`, so execution order never affects correctness — only
    // safety. The semaphore keeps a single `join_all` code path either way.
    let any_unsafe = tool_tuples.iter().any(|(tool, _)| !tool.concurrency_safe());
    let max_parallel = if any_unsafe {
        1
    } else {
        tool_tuples.len().max(1)
    };
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_parallel));

    let results = futures::future::join_all(tool_tuples.iter().map(|tuple| {
        let context = context.clone();
        let step_id = step_id.clone();
        let external_tool_calls_store = external_tool_calls_store.clone();
        let pending_receivers = pending_receivers.clone();
        let semaphore = semaphore.clone();
        async move {
            // Hold a permit for the duration of this tool's execution. With
            // `max_parallel == 1` (a batch containing a mutating tool) this
            // serializes the batch; otherwise all permits are available and
            // the batch runs fully in parallel.
            let _permit = semaphore.acquire().await.ok();
            let (tool, tool_call) = tuple;

            // Dry-run mode: simulate external and unsafe tools via LLM
            if context.dry_run
                && (tool.is_external()
                    || !crate::tools::simulator::is_safe_tool(&tool_call.tool_name))
            {
                context
                    .emit(AgentEventType::ToolExecutionStart {
                        step_id: step_id.clone(),
                        tool_call_id: tool_call.tool_call_id.clone(),
                        tool_call_name: tool_call.tool_name.clone(),
                        input: tool_call.input.clone(),
                    })
                    .await;

                tracing::info!(
                    tool = %tool_call.tool_name,
                    external = tool.is_external(),
                    "Dry-run: simulating tool response via LLM"
                );
                let def = tool.get_tool_definition();
                let parts = crate::tools::simulator::simulate_tool_response(
                    tool_call,
                    &def.description,
                    &def.parameters,
                )
                .await
                .unwrap_or_else(|e| vec![Part::Text(format!("Simulation error: {e}"))]);

                context
                    .emit(AgentEventType::ToolExecutionEnd {
                        step_id: step_id.clone(),
                        tool_call_id: tool_call.tool_call_id.clone(),
                        tool_call_name: tool_call.tool_name.clone(),
                        success: true,
                    })
                    .await;

                return ToolResultWithSkip::ToolResult(crate::types::ToolResponse::from_parts(
                    tool_call.tool_call_id.clone(),
                    tool_call.tool_name.clone(),
                    parts,
                ));
            }

            if tool.is_external() {
                let pre_rx = pending_receivers
                    .lock()
                    .await
                    .remove(&tool_call.tool_call_id);
                return handle_external_tool_inline(
                    external_tool_calls_store.clone(),
                    tool_call.clone(),
                    context.clone(),
                    &step_id,
                    timeout,
                    pre_rx,
                )
                .await;
            }

            context
                .emit(AgentEventType::ToolExecutionStart {
                    step_id: step_id.clone(),
                    tool_call_id: tool_call.tool_call_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                    input: tool_call.input.clone(),
                })
                .await;

            // Dry-run mode for non-external unsafe tools (shouldn't reach here often)
            if context.dry_run && !crate::tools::simulator::is_safe_tool(&tool_call.tool_name) {
                tracing::info!(
                    tool = %tool_call.tool_name,
                    "Dry-run: simulating tool response via LLM"
                );
                let def = tool.get_tool_definition();
                let parts = crate::tools::simulator::simulate_tool_response(
                    tool_call,
                    &def.description,
                    &def.parameters,
                )
                .await
                .unwrap_or_else(|e| vec![Part::Text(format!("Simulation error: {e}"))]);

                context
                    .emit(AgentEventType::ToolExecutionEnd {
                        step_id: step_id.clone(),
                        tool_call_id: tool_call.tool_call_id.clone(),
                        tool_call_name: tool_call.tool_name.clone(),
                        success: true,
                    })
                    .await;

                return ToolResultWithSkip::ToolResult(crate::types::ToolResponse::from_parts(
                    tool_call.tool_call_id.clone(),
                    tool_call.tool_name.clone(),
                    parts,
                ));
            }

            // Execute the tool based on its type
            let (parts, success) = if tool.needs_executor_context() {
                // ExecutorContext-based tool
                match execute_executor_context_tool(
                    tool.as_ref(),
                    tool_call.clone(),
                    context.clone(),
                )
                .await
                {
                    Ok(parts) => (parts, true),
                    Err(e) => (vec![Part::Text(e.to_string())], false),
                }
            } else {
                // ToolContext-based tool
                let tool_context = crate::tools::context::to_tool_context(context.as_ref());
                match tool
                    .execute(tool_call.clone(), Arc::new(tool_context))
                    .await
                {
                    Ok(parts) => (parts, true),
                    Err(e) => (vec![Part::Text(e.to_string())], false),
                }
            };
            // Emit completion event
            context
                .emit(AgentEventType::ToolExecutionEnd {
                    step_id: step_id.clone(),
                    tool_call_id: tool_call.tool_call_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                    success,
                })
                .await;
            // Wrap parts into ToolResponse
            ToolResultWithSkip::ToolResult(crate::types::ToolResponse::from_parts(
                tool_call.tool_call_id.clone(),
                tool_call.tool_name.clone(),
                parts,
            ))
        }
    }))
    .await;

    Ok(results)
}

/// Handle external tool execution with inline behavior - waits for response from client.
///
/// `pre_registered_rx` is the receiver produced during the pre-registration
/// pass in `execute_tool_calls_with_timeout`. When present, we skip the
/// late `register_external_tool_call` call (and the race it opens); when
/// absent (e.g. the pre-registration step failed or a caller outside the
/// standard path invokes this), we fall back to registering here.
async fn handle_external_tool_inline(
    store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    tool_call: crate::types::ToolCall,
    context: Arc<ExecutorContext>,
    step_id: &str,
    timeout: Duration,
    pre_registered_rx: Option<tokio::sync::oneshot::Receiver<distri_types::ToolResponse>>,
) -> ToolResultWithSkip {
    tracing::info!(
        target: "ext_tool.wait",
        task_id = %context.task_id,
        agent_id = %context.agent_id,
        parent_task_id = %context.parent_task_id.as_deref().unwrap_or("-"),
        tool_call_id = %tool_call.tool_call_id,
        tool_name = %tool_call.tool_name,
        timeout_s = timeout.as_secs(),
        "waiting for browser to complete external tool"
    );

    // Use tool_call_id as the session ID
    let tool_call_id = tool_call.tool_call_id.clone();

    let rx = match pre_registered_rx {
        Some(rx) => rx,
        None => match store.register_external_tool_call(&tool_call_id).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!("Failed to register external tool call: {}", e);
                return ToolResultWithSkip::Skip {
                    tool_call_id: tool_call.tool_call_id.clone(),
                    reason: format!("Failed to register external tool call: {}", e),
                };
            }
        },
    };

    // Emit tool execution start event
    context
        .emit(AgentEventType::ToolExecutionStart {
            step_id: step_id.to_string(),
            tool_call_id: tool_call.tool_call_id.clone(),
            tool_call_name: tool_call.tool_name.clone(),
            input: tool_call.input.clone(),
        })
        .await;

    // Wait for response with timeout
    let result = tokio::time::timeout(timeout, rx).await;

    match result {
        Ok(Ok(tool_response)) => {
            // Got a response from the client
            tracing::info!(
                target: "ext_tool.resolve",
                task_id = %context.task_id,
                agent_id = %context.agent_id,
                parent_task_id = %context.parent_task_id.as_deref().unwrap_or("-"),
                tool_call_id = %tool_call_id,
                tool_name = %tool_call.tool_name,
                outcome = "ok",
                "browser delivered external tool response"
            );

            // Emit tool execution end event
            context
                .emit(AgentEventType::ToolExecutionEnd {
                    step_id: step_id.to_string(),
                    tool_call_id: tool_call.tool_call_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                    success: true,
                })
                .await;

            ToolResultWithSkip::ToolResult(tool_response)
        }
        Ok(Err(_)) => {
            // Channel was closed (sender dropped)
            tracing::warn!(
                target: "ext_tool.resolve",
                task_id = %context.task_id,
                agent_id = %context.agent_id,
                parent_task_id = %context.parent_task_id.as_deref().unwrap_or("-"),
                tool_call_id = %tool_call_id,
                tool_name = %tool_call.tool_name,
                outcome = "channel_closed",
                "external tool channel closed before browser responded"
            );

            // Clean up the session in the store
            if let Err(e) = store.remove_tool_call(&tool_call_id).await {
                tracing::warn!("Failed to clean up tool call {}: {}", tool_call_id, e);
            }

            // Emit tool execution end event with failure
            context
                .emit(AgentEventType::ToolExecutionEnd {
                    step_id: step_id.to_string(),
                    tool_call_id: tool_call.tool_call_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                    success: false,
                })
                .await;

            // Fall back to input required behavior
            ToolResultWithSkip::Skip {
                tool_call_id: tool_call.tool_call_id.clone(),
                reason: "External tool channel closed".to_string(),
            }
        }
        Err(_) => {
            // Timeout occurred
            tracing::warn!(
                target: "ext_tool.resolve",
                task_id = %context.task_id,
                agent_id = %context.agent_id,
                parent_task_id = %context.parent_task_id.as_deref().unwrap_or("-"),
                tool_call_id = %tool_call_id,
                tool_name = %tool_call.tool_name,
                outcome = "timeout",
                timeout_s = timeout.as_secs(),
                "external tool TIMED OUT — browser never responded"
            );

            // Clean up the session in the store
            if let Err(e) = store.remove_tool_call(&tool_call_id).await {
                tracing::warn!("Failed to clean up tool call {}: {}", tool_call_id, e);
            }

            // Emit tool execution end event with failure
            context
                .emit(AgentEventType::ToolExecutionEnd {
                    step_id: step_id.to_string(),
                    tool_call_id: tool_call.tool_call_id.clone(),
                    tool_call_name: tool_call.tool_name.clone(),
                    success: false,
                })
                .await;

            // Fall back to input required behavior
            ToolResultWithSkip::Skip {
                tool_call_id: tool_call.tool_call_id.clone(),
                reason: "External tool execution timeout".to_string(),
            }
        }
    }
}

/// Helper function to execute tools that need ExecutorContext, returning content parts
async fn execute_executor_context_tool(
    tool: &dyn Tool,
    tool_call: crate::types::ToolCall,
    context: Arc<ExecutorContext>,
) -> Result<Vec<Part>, AgentError> {
    use crate::tools::execute_tool_with_executor_context;

    execute_tool_with_executor_context(tool, tool_call, context).await
}

#[derive(Default)]
pub struct ToolResultResponse {
    pub tool_responses: Vec<ToolResponse>,
    pub input_required: bool,
}
