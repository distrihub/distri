use crate::{
    agent::{
        AgentEvent, AgentEventType, AgentExecutor, AgentType, BaseAgent, StepResult, MAX_ITERATIONS,
    },
    error::AgentError,
    llm::LLMExecutor,
    tools::{LlmToolsRegistry, Tool},
    types::{
        get_tool_descriptions, AgentDefinition, Message, MessageMetadata, MessagePart, MessageRole,
        PlanConfig, TaskStatus, ToolCall, DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
    SessionStore,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use crate::agent::{reason::create_initial_plan, ExecutorContext};

/// Standard agent implementation
#[derive(Clone)]
pub struct StandardAgent {
    pub definition: AgentDefinition,
    tools_registry: Arc<LlmToolsRegistry>, // This will be Arc::default() now
    executor: Arc<AgentExecutor>,
    #[allow(dead_code)]
    session_store: Arc<Box<dyn SessionStore>>,
}

impl std::fmt::Debug for StandardAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandardAgent")
            .field("definition", &self.definition)
            .field("tools_registry", &"Arc<LlmToolsRegistry>")
            .finish()
    }
}

impl StandardAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>, // pass Arc::default() here
        executor: Arc<AgentExecutor>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self {
            definition,
            tools_registry,
            executor,
            session_store,
        }
    }

    pub async fn plan_step(
        &self,
        message: Message,
        plan_config: &PlanConfig,
        current_messages: &mut Vec<Message>,
        iteration: usize,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let tools_desc = get_tool_descriptions(
            &self.tools_registry.tools,
            Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE),
        );

        info!(
            "plan_step: iteration: {}, plan_config: {:?}",
            iteration, plan_config
        );
        if (iteration - 1) % plan_config.interval as usize == 0 {
            // Run either initial planning or planning update
            let (facts, plan) = if iteration == 1 {
                create_initial_plan(&message, &tools_desc, &|msgs| {
                    let planning_executor = LLMExecutor::new(
                        crate::agent::reason::get_planning_definition(
                            plan_config.model_settings.clone(),
                        ),
                        Arc::default(),
                        context.clone(),
                        None,
                        Some("initial_plan".to_string()),
                    );
                    Box::pin(async move {
                        let response = planning_executor.execute(&msgs).await;
                        match response {
                            Ok(response) => {
                                // Extract just the content string
                                let content = response.content.clone();
                                Ok(content)
                            }
                            Err(e) => {
                                tracing::error!("Planning execution failed: {}", e);
                                Ok(format!("Planning execution failed: {}", e))
                            }
                        }
                    })
                })
                .await
            } else {
                let remaining_steps =
                    plan_config.max_iterations.unwrap_or(10) - iteration + 1 as usize;

                crate::agent::reason::update_plan(
                    &message,
                    &tools_desc,
                    &current_messages,
                    remaining_steps,
                    &|msgs| {
                        let planning_executor = LLMExecutor::new(
                            crate::agent::reason::get_planning_definition(
                                plan_config.model_settings.clone(),
                            ),
                            Arc::default(),
                            context.clone(),
                            None,
                            Some("update_plan".to_string()),
                        );
                        Box::pin(async move {
                            let response = planning_executor.execute(&msgs).await;
                            match response {
                                Ok(response) => {
                                    // Extract just the content string
                                    let content = response.content.clone();
                                    Ok(content)
                                }
                                Err(e) => {
                                    tracing::error!("Planning execution failed: {}", e);
                                    Ok(format!("Planning execution failed: {}", e))
                                }
                            }
                        })
                    },
                )
                .await
            }
            .map_err(|e| AgentError::Session(e.to_string()))?;

            // Store planning step
            self.add_messages_to_current_messages(
                &[
                    Message {
                        role: MessageRole::Assistant,
                        metadata: Some(MessageMetadata::PlanFacts {
                            facts: facts.clone(),
                        }),
                        ..Default::default()
                    },
                    Message {
                        role: MessageRole::Assistant,
                        parts: vec![MessagePart::Text(plan.clone())],
                        ..Default::default()
                    },
                ],
                current_messages,
                context.clone(),
            )
            .await?;
        }

        Ok(())
    }

    async fn system_step(&self, context: Arc<ExecutorContext>) -> Result<Message, AgentError> {
        // Store system message if present

        let message = Message::system(self.definition.system_prompt.clone(), None);
        self.executor
            .task_store
            .add_message_to_task(&context.run_id, &message)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        Ok(message)
    }

    /// Execute one step using LLM
    async fn llm_step(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<AgentEvent>>,
        hooks: Option<&dyn crate::agent::AgentHooks>,
    ) -> Result<StepResult, AgentError> {
        let agent_id = &self.definition.name;

        // Create executor for LLM call
        let executor = LLMExecutor::new(
            self.definition.clone().into(),
            self.tools_registry.clone(),
            context.clone(),
            None,
            Some(format!("{}:{}", agent_id, "step")),
        );

        // Execute LLM call
        let response = executor.execute(messages).await?;

        let response = if let Some(hooks) = hooks {
            hooks.after_execute(response).await?
        } else {
            response
        };

        let step_result = self
            .handle_finish_reason(
                response.finish_reason,
                response.content,
                response.tool_calls,
            )
            .await?;

        Ok(step_result)
    }

    /// Execute one step using LLM
    async fn llm_step_stream(
        &self,
        messages: &[Message],
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
        hooks: Option<&dyn crate::agent::AgentHooks>,
    ) -> Result<StepResult, AgentError> {
        let agent_id = &self.definition.name;

        let executor = LLMExecutor::new(
            self.definition.clone().into(),
            self.tools_registry.clone(),
            context.clone(),
            None,
            Some(agent_id.to_string()),
        );
        // Streaming LLM step: propagate deltas via event_tx
        let stream_result = executor.execute_stream(&messages, event_tx.clone()).await?;
        let stream_result = if let Some(hooks) = hooks {
            hooks.after_execute_stream(stream_result).await?
        } else {
            stream_result
        };

        let step_result = self
            .handle_finish_reason(
                stream_result.finish_reason,
                stream_result.content,
                stream_result.tool_calls,
            )
            .await?;

        Ok(step_result)
    }

    pub async fn invoke_with_hooks(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
        hooks: Option<&dyn crate::agent::AgentHooks>,
    ) -> Result<String, AgentError> {
        let run_id = context.run_id.clone();
        let thread_id = context.thread_id.clone();

        if let Some(hooks) = hooks {
            hooks
                .before_invoke(message.clone(), context.clone(), event_tx.clone())
                .await?;
        }

        // Send RunStarted event if event_tx is provided
        if let Some(event_tx) = &event_tx {
            let _ = event_tx
                .send(AgentEvent {
                    thread_id: thread_id.clone(),
                    run_id: run_id.clone(),
                    event: AgentEventType::RunStarted {},
                })
                .await;
        }
        let history = self
            .executor
            .task_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        let mut iterations = history.len();
        self.executor
            .task_store
            .update_task_status(&context.run_id, TaskStatus::Running)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        let result = async {
            let mut current_messages = vec![];
            if iterations == 0 {
                self.add_messages_to_current_messages(
                    &[self.system_step(context.clone()).await?, message],
                    &mut current_messages,
                    context.clone(),
                )
                .await?;
            }
            current_messages.extend(history);

            let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS) as usize;
            tracing::debug!("Max iterations per run set to: {}", max_iterations);
            loop {
                if iterations >= max_iterations {
                    tracing::warn!("Max iterations limit reached: {}", max_iterations);
                    return Err(AgentError::LLMError(format!(
                        "Max iterations reached: {max_iterations}",
                    )));
                }

                current_messages = if let Some(hooks) = hooks {
                    hooks.llm_messages(&current_messages).await?
                } else {
                    current_messages
                };

                let step_result = self
                    .llm_step(&current_messages, context.clone(), event_tx.clone(), hooks)
                    .await?;

                let step_result = if let Some(hooks) = hooks {
                    hooks.before_step_result(step_result).await?
                } else {
                    step_result
                };
                self.handle_step_result(
                    &step_result,
                    &mut current_messages,
                    context.clone(),
                    event_tx.clone(),
                    hooks,
                )
                .await?;
                if let StepResult::Finish(content) = &step_result {
                    break Ok(content.clone());
                }
                iterations += 1;
            }
        }
        .await;
        if let Some(event_tx) = &event_tx {
            match &result {
                Ok(_) => {
                    let _ = event_tx
                        .send(AgentEvent {
                            thread_id: thread_id.clone(),
                            run_id: run_id.clone(),
                            event: AgentEventType::RunFinished {},
                        })
                        .await;
                    self.update_task_status(context.clone(), TaskStatus::Completed)
                        .await?;
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AgentEvent {
                            thread_id: thread_id.clone(),
                            run_id: run_id.clone(),
                            event: AgentEventType::RunError {
                                message: e.to_string(),
                                code: None,
                            },
                        })
                        .await;
                    self.update_task_status(context.clone(), TaskStatus::Failed)
                        .await?;
                }
            }
        }
        result
    }

    pub async fn update_task_status(
        &self,
        context: Arc<ExecutorContext>,
        status: TaskStatus,
    ) -> Result<(), AgentError> {
        self.executor
            .task_store
            .update_task_status(&context.run_id, status)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn add_messages_to_current_messages(
        &self,
        messages: &[Message],
        current_messages: &mut Vec<Message>,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        for m in messages {
            self.executor
                .task_store
                .add_message_to_task(&context.run_id, m)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
        }
        current_messages.extend(messages.to_vec());
        Ok(())
    }

    pub async fn handle_step_result(
        &self,
        step_result: &StepResult,
        current_messages: &mut Vec<Message>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
        hooks: Option<&dyn crate::agent::AgentHooks>,
    ) -> Result<(), AgentError> {
        let agent_id = &self.definition.name;
        match step_result {
            StepResult::Finish(content) => {
                // Store final response as action step
                self.add_messages_to_current_messages(
                    &[Message {
                        role: MessageRole::Assistant,
                        name: Some(agent_id.to_string()),
                        parts: vec![MessagePart::Text(content.clone())],
                        metadata: Some(MessageMetadata::FinalResponse {
                            final_response: true,
                        }),
                        ..Default::default()
                    }],
                    current_messages,
                    context.clone(),
                )
                .await?;
                Ok(())
            }

            StepResult::ToolCalls(tool_calls) => {
                // Convert and add assistant message with tool calls
                let new_messages = &[Message {
                    role: MessageRole::Assistant,
                    name: Some(agent_id.to_string()),
                    metadata: Some(MessageMetadata::ToolCalls {
                        tool_calls: tool_calls.clone(),
                    }),
                    ..Default::default()
                }];
                self.add_messages_to_current_messages(
                    new_messages,
                    current_messages,
                    context.clone(),
                )
                .await?;

                let tool_calls = if let Some(hooks) = hooks {
                    hooks.before_tool_calls(&tool_calls).await?
                } else {
                    tool_calls.clone()
                };

                let tool_responses = execute_tool_calls(
                    self.executor.clone(),
                    tool_calls,
                    agent_id,
                    context.clone(),
                    event_tx.clone(),
                )
                .await?;

                let tool_responses = if let Some(hooks) = hooks {
                    hooks.after_tool_calls(&tool_responses).await?
                } else {
                    tool_responses
                };

                // Add tool responses
                self.add_messages_to_current_messages(
                    &tool_responses,
                    current_messages,
                    context.clone(),
                )
                .await?;
                Ok(())
            }
        }
    }
    pub async fn invoke_stream_with_hooks(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
        hooks: Option<&dyn crate::agent::AgentHooks>,
    ) -> Result<(), AgentError> {
        let agent_id = &self.definition.name;
        let run_id = context.run_id.clone();
        let thread_id = context.thread_id.clone();
        let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS) as usize;

        if let Some(hooks) = hooks {
            hooks
                .before_invoke(message.clone(), context.clone(), Some(event_tx.clone()))
                .await?;
        }
        // Send RunStarted event
        let _ = event_tx
            .send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::RunStarted {},
            })
            .await;

        let history = self
            .executor
            .task_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        let mut iterations = history.len();

        let result = async {
            tracing::info!(
                "Invoking stream for agent: {}, Iterations: {}",
                agent_id,
                iterations
            );
            let mut current_messages = vec![];
            if iterations == 0 {
                self.add_messages_to_current_messages(
                    &[self.system_step(context.clone()).await?, message.clone()],
                    &mut current_messages,
                    context.clone(),
                )
                .await?;
            }
            current_messages.extend(history);

            println!("{:?}", current_messages);
            loop {
                if iterations > max_iterations {
                    return Err(AgentError::LLMError(format!(
                        "Max iterations reached: {max_iterations}",
                    )));
                }
                // Handle planning if enabled
                if let Some(plan_config) = &self.definition.plan {
                    self.plan_step(
                        message.clone(),
                        plan_config,
                        &mut current_messages,
                        iterations,
                        context.clone(),
                    )
                    .await?;
                }
                current_messages = if let Some(hooks) = hooks {
                    hooks.llm_messages(&current_messages).await?
                } else {
                    current_messages
                };

                println!("current_messages after hooks: {:?}", current_messages);
                let step_result = self
                    .llm_step_stream(&current_messages, context.clone(), event_tx.clone(), hooks)
                    .await?;

                let step_result = if let Some(hooks) = hooks {
                    hooks.before_step_result(step_result).await?
                } else {
                    step_result
                };

                self.handle_step_result(
                    &step_result,
                    &mut current_messages,
                    context.clone(),
                    Some(event_tx.clone()),
                    hooks,
                )
                .await?;

                if let StepResult::Finish(content) = &step_result {
                    return Ok(content.clone());
                }

                iterations += 1;
            }
        }
        .await;

        // Send RunFinished or RunError event
        match &result {
            Ok(_) => {
                let _ = event_tx
                    .send(AgentEvent {
                        thread_id: thread_id.clone(),
                        run_id: run_id.clone(),
                        event: AgentEventType::RunFinished {},
                    })
                    .await;
                self.update_task_status(context.clone(), TaskStatus::Completed)
                    .await?;
            }
            Err(e) => {
                let _ = event_tx
                    .send(AgentEvent {
                        thread_id: thread_id.clone(),
                        run_id: run_id.clone(),
                        event: AgentEventType::RunError {
                            message: e.to_string(),
                            code: None,
                        },
                    })
                    .await;
                self.update_task_status(context.clone(), TaskStatus::Failed)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_finish_reason(
        &self,
        finish_reason: async_openai::types::FinishReason,
        content: String,
        tool_calls: Vec<ToolCall>,
    ) -> Result<StepResult, AgentError> {
        match finish_reason {
            async_openai::types::FinishReason::Stop => {
                // Return finish result
                Ok(StepResult::Finish(content))
            }
            async_openai::types::FinishReason::ToolCalls => {
                if !tool_calls.is_empty() {
                    Ok(StepResult::ToolCalls(tool_calls))
                } else {
                    Err(AgentError::LLMError(
                        "Tool calls finish reason but no tool calls".to_string(),
                    ))
                }
            }
            x => Err(AgentError::LLMError(format!(
                "Unexpected finish reason: {:?}",
                x
            ))),
        }
    }
}

#[async_trait::async_trait]
impl BaseAgent for StandardAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Standard
    }

    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_tools(&self) -> Vec<&Box<dyn Tool>> {
        self.tools_registry.tools.values().collect()
    }

    async fn invoke(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        StandardAgent::invoke_with_hooks(self, message, context, event_tx, None).await
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(StandardAgent::clone(self))
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    async fn invoke_stream(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        StandardAgent::invoke_stream_with_hooks(self, message, context, event_tx, None).await
    }
}

pub async fn execute_tool_calls(
    executor: Arc<AgentExecutor>,
    tool_calls: Vec<ToolCall>,
    agent_id: &str,
    context: Arc<ExecutorContext>,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
) -> Result<Vec<Message>, AgentError> {
    // Get agent definition to check approval requirements
    let definition = executor
        .agent_store
        .get(agent_id)
        .await
        .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

    let agent = executor.create_agent_from_definition(definition.clone()).await?;
    let tools_registry = agent.get_tools();

    // Separate built-in tools from external tools
    let mut built_in_tool_calls = Vec::new();
    let mut external_tool_calls = Vec::new();
    let mut approval_required_tool_calls = Vec::new();

    for tool_call in tool_calls {
        if let Some(tool) = tools_registry.iter().find(|t| t.get_name() == tool_call.tool_name) {
            // Check if this is an external tool
            if tool.get_name().starts_with("external_") || tool.get_name().contains("frontend") {
                external_tool_calls.push(tool_call);
            } else {
                // Check if approval is required for this tool
                let requires_approval = tools_registry
                    .iter()
                    .find(|t| t.get_name() == tool_call.tool_name)
                    .map(|_| {
                        // This is a bit of a hack since we don't have direct access to the registry
                        // We'll check the agent definition directly
                        if let Some(approval_config) = &definition.tool_approval {
                            if approval_config.approval_required {
                                if approval_config.use_whitelist {
                                    !approval_config.approval_whitelist.contains(&tool_call.tool_name)
                                } else {
                                    approval_config.approval_blacklist.contains(&tool_call.tool_name)
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);

                if requires_approval {
                    approval_required_tool_calls.push(tool_call);
                } else {
                    built_in_tool_calls.push(tool_call);
                }
            }
        } else {
            // Unknown tool - treat as external
            external_tool_calls.push(tool_call);
        }
    }

    let mut all_responses = Vec::new();

    // Handle approval-required tools first
    if !approval_required_tool_calls.is_empty() {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let approval_message = Message {
            role: MessageRole::Assistant,
            name: Some(agent_id.to_string()),
            metadata: Some(MessageMetadata::ToolApprovalRequest {
                tool_calls: approval_required_tool_calls.clone(),
                approval_id: approval_id.clone(),
                reason: Some("Tool execution requires approval".to_string()),
            }),
            ..Default::default()
        };
        all_responses.push(approval_message);

        // For now, we'll wait for approval by returning early
        // In a real implementation, you'd want to handle this asynchronously
        return Ok(all_responses);
    }

    // Handle external tools
    if !external_tool_calls.is_empty() {
        let external_message = Message {
            role: MessageRole::Assistant,
            name: Some(agent_id.to_string()),
            metadata: Some(MessageMetadata::ExternalToolCalls {
                tool_calls: external_tool_calls.clone(),
                requires_approval: false,
            }),
            ..Default::default()
        };
        all_responses.push(external_message);
    }

    // Process built-in tools in parallel
    if !built_in_tool_calls.is_empty() {
        let tool_responses = futures::future::join_all(built_in_tool_calls.iter().map(|mapped_tool_call| {
            let executor = executor.clone();
            let agent_id = agent_id.to_string();
            let context = context.clone();
            let event_tx = event_tx.clone();

            async move {
                let run_id = { context.run_id.clone() };
                if let Some(event_tx) = &event_tx {
                    let _ = event_tx
                        .send(AgentEvent {
                            thread_id: context.thread_id.clone(),
                            run_id: run_id.clone(),
                            event: AgentEventType::ToolCallStart {
                                tool_call_id: mapped_tool_call.tool_id.clone(),
                                tool_call_name: mapped_tool_call.tool_name.clone(),
                            },
                        })
                        .await
                        .map_err(|e| {
                            AgentError::LLMError(format!("Failed to send ToolCallStart event: {}", e))
                        });

                    let _ = event_tx
                        .send(AgentEvent {
                            thread_id: context.thread_id.clone(),
                            run_id: run_id.clone(),
                            event: AgentEventType::ToolCallArgs {
                                tool_call_id: mapped_tool_call.tool_id.clone(),
                                delta: mapped_tool_call.input.clone(),
                            },
                        })
                        .await
                        .map_err(|e| {
                            AgentError::LLMError(format!("Failed to send ToolCallStart event: {}", e))
                        });
                }
                info!("Agent: Executing tool call: {:#?}", mapped_tool_call);
                let content = executor
                    .execute_tool(
                        agent_id,
                        mapped_tool_call.clone(),
                        event_tx.clone(),
                        context.clone(),
                    )
                    .await
                    .unwrap_or_else(|err| format!("Error: {}", err));
                info!("Agent: Tool response: {}", content);

                if let Some(event_tx) = &event_tx {
                    let _ = event_tx
                        .send(AgentEvent {
                            thread_id: context.thread_id.clone(),
                            run_id: run_id.clone(),
                            event: AgentEventType::ToolCallResult {
                                tool_call_id: mapped_tool_call.tool_id.clone(),
                                result: content.clone(),
                            },
                        })
                        .await
                        .map_err(|e| {
                            AgentError::LLMError(format!("Failed to send ToolCallResult event: {}", e))
                        });
                }
                Message {
                    role: MessageRole::User,
                    name: Some(mapped_tool_call.tool_name.clone()),
                    metadata: Some(MessageMetadata::ToolResponse {
                        tool_call_id: mapped_tool_call.tool_id.clone(),
                        result: content.clone(),
                    }),
                    ..Default::default()
                }
            }
        }))
        .await;

        all_responses.extend(tool_responses);
    }

    Ok(all_responses)
}
