#[cfg(feature = "code")]
use crate::tools::DistriExecuteCodeTool;
use crate::{
    agent::{
        file::process_large_tool_responses,
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

        // Emit tool call event
        context
            .emit(AgentEventType::ToolCalls {
                step_id: step_id.to_string(),
                parent_message_id: context.get_current_message_id().await,
                tool_calls: tool_calls.to_vec(),
            })
            .await;

        #[allow(unused_mut)]
        // Get all available tools from context (including MCP tools)
        let mut enhanced_tools = context.get_tools().await;

        // Add code execution tools if we have a distri_execute_code call
        let has_code_execution = tool_calls
            .iter()
            .any(|tc| tc.tool_name == "distri_execute_code");

        if has_code_execution {
            #[cfg(feature = "code")]
            {
                enhanced_tools.push(Arc::new(DistriExecuteCodeTool) as Arc<dyn Tool>);
                tracing::debug!("Added DistriExecuteCodeTool for code execution");
            }
            #[cfg(not(feature = "code"))]
            {
                tracing::warn!("DistriExecuteCodeTool requested but 'code' feature is disabled");
            }
        }

        // Get external tool timeout from agent definition strategy
        let external_tool_timeout_secs = self
            .agent_definition
            .as_ref()
            .and_then(|def| def.strategy.as_ref())
            .map(|s| s.get_external_tool_timeout_secs())
            .unwrap_or(DEFAULT_EXTERNAL_TOOL_TIMEOUT_SECS);

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
                    parts.extend(tools_response.parts);
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
        step: &PlanStep,
    ) -> Result<ToolResultResponse, AgentError> {
        if tool_results.is_empty() {
            return Ok(ToolResultResponse::default());
        }

        let mut processed_tool_results: Vec<crate::types::ToolResponse> = Vec::new();
        let orchestrator = context.get_orchestrator()?;

        let mut input_required = false;
        for result in tool_results {
            match result {
                ToolResultWithSkip::ToolResult(tool_result) => {
                    let should_process_artifacts = self
                        .agent_definition
                        .as_ref()
                        .map(|def| def.should_write_large_tool_responses_to_fs())
                        .unwrap_or(false);

                    let processed_response = if should_process_artifacts {
                        // Get the original task from the current step's thought
                        let original_task = step
                            .thought
                            .clone()
                            .unwrap_or_else(|| "Analyze the content".to_string());

                        // Process tool response through filesystem's artifact wrapper to handle large content
                        process_large_tool_responses(
                            tool_result.clone(),
                            &context.thread_id,
                            &context.task_id,
                            &orchestrator,
                            &original_task,
                        )
                        .await
                        .map_err(|e| AgentError::ToolResponseProcessing(e.to_string()))?
                    } else {
                        // Return the tool response as-is without artifact processing
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

        let flattened_parts = processed_tool_results
            .iter()
            .flat_map(|result| result.parts.clone())
            .collect();

        Ok(ToolResultResponse {
            parts: flattened_parts,
            tool_responses: processed_tool_results,
            input_required,
        })
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
            #[allow(unused)]
            Action::Code { code, .. } => {
                #[cfg(feature = "code")]
                {
                    // Execute code and handle errors gracefully - centralized tool setup
                    match crate::tools::execute_code_with_tools(code, context.clone()).await {
                        Ok((_, observations, _)) => Ok(ExecutionResult {
                            step_id: step.id.clone(),
                            status: ExecutionStatus::Success,
                            parts: vec![Part::Text(observations.join("\n\n"))],
                            timestamp: chrono::Utc::now().timestamp_millis(),
                            reason: None,
                        }),
                        Err(e) => {
                            // Code execution failed - capture error but continue
                            tracing::warn!("Direct code execution failed: {}", e);
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
                #[cfg(not(feature = "code"))]
                {
                    tracing::warn!("Code execution requested but 'code' feature is disabled");
                    Ok(ExecutionResult {
                        step_id: step.id.clone(),
                        status: ExecutionStatus::Failed,
                        parts: vec![],
                        timestamp: chrono::Utc::now().timestamp_millis(),
                        reason: Some("Code execution feature disabled".to_string()),
                    })
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
        context.get_final_result().await.is_none()
            && matches!(
                context.get_status().await,
                Some(crate::types::TaskStatus::Running)
            )
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

    let step_id = step_id.to_string();
    let timeout = Duration::from_secs(external_tool_timeout_secs);
    let results = futures::future::join_all(tool_tuples.iter().map(|tuple| {
        let context = context.clone();
        let step_id = step_id.clone();
        let external_tool_calls_store = external_tool_calls_store.clone();
        async move {
            let (tool, tool_call) = tuple;

            if tool.is_external() {
                return handle_external_tool_inline(
                    external_tool_calls_store.clone(),
                    tool_call.clone(),
                    context.clone(),
                    &step_id,
                    timeout,
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
            // Execute the tool based on its type
            // Execute the tool, obtaining content parts
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

/// Handle external tool execution with inline behavior - waits for response from client
async fn handle_external_tool_inline(
    store: Arc<dyn distri_types::stores::ExternalToolCallsStore>,
    tool_call: crate::types::ToolCall,
    context: Arc<ExecutorContext>,
    step_id: &str,
    timeout: Duration,
) -> ToolResultWithSkip {
    tracing::info!(
        "Waiting for tool response: {}, {} (timeout: {}s)",
        tool_call.tool_name,
        tool_call.tool_call_id,
        timeout.as_secs()
    );

    // Use tool_call_id as the session ID
    let tool_call_id = tool_call.tool_call_id.clone();

    // Register the external tool call and get receiver
    let rx = match store.register_external_tool_call(&tool_call_id).await {
        Ok(rx) => rx,
        Err(e) => {
            tracing::error!("Failed to register external tool call: {}", e);
            return ToolResultWithSkip::Skip {
                tool_call_id: tool_call.tool_call_id.clone(),
                reason: format!("Failed to register external tool call: {}", e),
            };
        }
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
            tracing::debug!(
                "Received external tool response for tool call: {}",
                tool_call_id
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
                "External tool channel closed for tool call: {}",
                tool_call_id
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
            tracing::warn!("External tool timeout for tool call: {}", tool_call_id);

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
    pub parts: Vec<Part>,
    pub tool_responses: Vec<ToolResponse>,
    pub input_required: bool,
}
