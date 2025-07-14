use crate::{
    agent::{AgentEvent, AgentEventType, AgentExecutor, AgentHooks},
    error::AgentError,
    llm::LLMExecutor,
    memory::SystemStep,
    tools::{LlmToolsRegistry, Tool},
    types::{
        get_tool_descriptions, AgentDefinition, Message, MessageContent, MessageRole, PlanConfig,
        ToolCall, DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
    SessionStore,
};
use async_openai::types::Role;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

use crate::agent::{reason::create_initial_plan, ExecutorContext, StepLogger};
use crate::memory::{ActionStep, MemoryStep, PlanningStep, TaskStep};

pub const MAX_ITERATIONS: i32 = 10;

#[async_trait::async_trait]
pub trait BaseAgent: Send + Sync + std::fmt::Debug {
    async fn validate(&self) -> Result<(), AgentError> {
        self.get_definition()
            .validate()
            .map_err(|e| AgentError::Validation(e.to_string()))?;
        Ok(())
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError>;

    async fn invoke_stream(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        Err(AgentError::NotImplemented(
            "BaseAgent::invoke_stream not implemented".to_string(),
        ))
    }

    /// Continue execution with tool responses (for external tool calls)
    async fn continue_execution(
        &self,
        _tool_response_messages: Vec<Message>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        Err(AgentError::NotImplemented(
            "BaseAgent::continue_execution not implemented".to_string(),
        ))
    }

    /// Clone the agent (required for object safety)
    fn clone_box(&self) -> Box<dyn BaseAgent>;

    /// Get the agent's name/id
    fn get_name(&self) -> &str;

    fn get_description(&self) -> &str;
    fn get_definition(&self) -> AgentDefinition;
    fn get_tools(&self) -> Vec<&Box<dyn Tool>>;

    // Used in deserialization
    fn agent_type(&self) -> AgentType;
}

/// Result of a single step execution
#[derive(Debug)]
pub enum StepResult {
    /// Continue with more iterations, with new messages to add
    Continue(Vec<Message>),
    /// Finish execution with this final response
    Finish(String),
}

/// Standard agent implementation that provides the full LLM-based behavior with extensible hooks
#[derive(Clone)]
pub struct StandardAgent {
    pub definition: AgentDefinition,
    tools_registry: Arc<LlmToolsRegistry>,
    executor: Arc<AgentExecutor>,
    logger: StepLogger,
    session_store: Arc<Box<dyn SessionStore>>,
}

impl std::fmt::Debug for StandardAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandardAgent")
            .field("definition", &self.definition)
            .finish()
    }
}

impl StandardAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        executor: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let logger = StepLogger::new(context.verbose);
        Self {
            definition,
            tools_registry,
            executor,
            logger,
            session_store,
        }
    }

    pub async fn plan_step(
        &self,
        task: TaskStep,
        plan_config: &PlanConfig,
        iteration: i32,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let agent_id = &self.definition.name;
        let tools_desc = get_tool_descriptions(
            &self.tools_registry.tools,
            Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE),
        );

        println!(
            "plan_step: iteration: {}, plan_config: {:?}",
            iteration, plan_config
        );
        if (iteration - 1) % plan_config.interval == 0 {
            // Run either initial planning or planning update
            let (facts, plan) = if iteration == 1 {
                create_initial_plan(&task, &tools_desc, &|msgs| {
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
                        let response = planning_executor.execute(&msgs, None).await;
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
                let remaining_steps = plan_config.max_iterations.unwrap_or(10) - iteration + 1;
                let previous_messages = self
                    .session_store
                    .get_messages(&context.thread_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?;
                crate::agent::reason::update_plan(
                    &task.task,
                    &tools_desc,
                    &previous_messages,
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
                            let response = planning_executor.execute(&msgs, None).await;
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
            let planning_step = MemoryStep::Planning(PlanningStep {
                model_input_messages: vec![],
                model_output_message_facts: Message {
                    role: MessageRole::Assistant,
                    name: Some("planner".to_string()),
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(facts.clone()),
                        image: None,
                    }],
                    tool_calls: Vec::new(),
                },
                facts: facts.clone(),
                model_output_message_plan: Message {
                    role: MessageRole::Assistant,
                    name: Some("planner".to_string()),
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(plan.clone()),
                        image: None,
                    }],
                    tool_calls: Vec::new(),
                },
                plan: plan.clone(),
            });
            self.session_store
                .store_step(&context.thread_id, planning_step.clone())
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &planning_step);
        }

        Ok(())
    }

    async fn system_step(&self, context: Arc<ExecutorContext>) -> Result<(), AgentError> {
        let agent_id = &self.definition.name;
        // Store system message if present
        if let Some(system_prompt) = &self.definition.system_prompt {
            let step = MemoryStep::System(SystemStep {
                system_prompt: system_prompt.clone(),
            });
            self.session_store
                .store_step(&context.thread_id, step.clone())
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &step);
        }
        Ok(())
    }

    async fn task_step(
        &self,
        task: &TaskStep,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let task_step = MemoryStep::Task(task.clone());
        let agent_id = &self.definition.name;
        self.session_store
            .store_step(&context.thread_id, task_step.clone())
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        self.logger.log_step(agent_id, &task_step);
        Ok(())
    }

    /// Execute one step using LLM
    async fn llm_step(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
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
        let response = executor.execute(messages, params.clone()).await?;

        let step_result = self
            .handle_finish_reason(
                response.finish_reason,
                response.content,
                response.tool_calls,
                agent_id,
                context.clone(),
                event_tx.clone(),
            )
            .await?;

        Ok(step_result)
    }

    /// Execute one step using LLM
    async fn llm_step_stream(
        &self,
        messages: &[Message],
        params: &Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
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
        let stream_result = executor
            .execute_stream(&messages, params.clone(), event_tx.clone())
            .await?;

        let step_result = self
            .handle_finish_reason(
                stream_result.finish_reason,
                stream_result.content,
                stream_result.tool_calls,
                agent_id,
                context.clone(),
                Some(event_tx.clone()),
            )
            .await?;

        Ok(step_result)
    }

    /// Helper to emit TextMessage* events for a message
    async fn emit_text_message_events(
        &self,
        event_tx: &mpsc::Sender<AgentEvent>,
        context: &ExecutorContext,
        content: &str,
        role: &Role,
    ) {
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();
        let message_id = Uuid::new_v4().to_string();
        let _ = event_tx
            .send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::TextMessageStart {
                    message_id: message_id.clone(),
                    role: role.clone(),
                },
            })
            .await;
        let _ = event_tx
            .send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::TextMessageContent {
                    message_id: message_id.clone(),
                    delta: content.to_string(),
                },
            })
            .await;
        let _ = event_tx
            .send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::TextMessageEnd {
                    message_id: message_id.clone(),
                },
            })
            .await;
    }

    pub async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        self.validate().await?;
        let agent_id = &self.definition.name;
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();
        let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS);
        let mut iterations = self
            .session_store
            .get_iteration(&run_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Send RunStarted event
        let _ = event_tx
            .send(AgentEvent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                event: AgentEventType::RunStarted {},
            })
            .await;

        let result = async {
            tracing::info!(
                "Invoking stream for agent: {}, Iterations: {}",
                agent_id,
                iterations
            );
            if iterations == 0 {
                self.system_step(context.clone()).await?;
                self.task_step(&task, context.clone()).await?;
                // Call after_task_step hook
                self.after_task_step(task.clone(), context.clone()).await?;
            }
            let mut current_messages = self
                .session_store
                .get_messages(&context.thread_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            loop {
                if iterations > max_iterations {
                    return Err(AgentError::LLMError(format!(
                        "Max iterations reached: {max_iterations}",
                    )));
                }
                // Handle planning if enabled
                if let Some(plan_config) = &self.definition.plan {
                    self.plan_step(task.clone(), plan_config, iterations, context.clone())
                        .await?;
                }
                let messages = self
                    .before_llm_step(&current_messages, &params, context.clone())
                    .await?;
                let step_result = self
                    .llm_step_stream(&messages, &params, context.clone(), event_tx.clone())
                    .await?;
                match step_result {
                    StepResult::Finish(content) => {
                        // Call after_finish hook
                        self.after_finish(&content, context.clone()).await?;

                        // Store final response as action step
                        let action_step = MemoryStep::Action(ActionStep {
                            model_input_messages: Some(current_messages),
                            model_output: Some(content.clone()),
                            ..Default::default()
                        });
                        self.session_store
                            .store_step(&context.thread_id, action_step.clone())
                            .await
                            .map_err(|e| AgentError::Session(e.to_string()))?;
                        self.logger.log_step(agent_id, &action_step);

                        break Ok(());
                    }
                    StepResult::Continue(new_messages) => {
                        current_messages.extend(new_messages);
                        iterations = self
                            .session_store
                            .inc_iteration(&run_id)
                            .await
                            .map_err(|e| AgentError::Session(e.to_string()))?;
                        continue;
                    }
                }
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
            }
        }
        result
    }

    pub async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.validate().await?;
        let agent_id = &self.definition.name;
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();
        let mut iterations = self
            .session_store
            .get_iteration(&run_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
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
        let result = async {
            if iterations == 0 {
                self.system_step(context.clone()).await?;
                self.task_step(&task, context.clone()).await?;
                // Call after_task_step hook
                self.after_task_step(task.clone(), context.clone()).await?;
            }
            let mut current_messages = self
                .session_store
                .get_messages(&context.thread_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS);
            tracing::debug!("Max iterations per run set to: {}", max_iterations);
            loop {
                if iterations >= max_iterations {
                    tracing::warn!("Max iterations limit reached: {}", max_iterations);
                    return Err(AgentError::LLMError(format!(
                        "Max iterations reached: {max_iterations}",
                    )));
                }
                iterations = self
                    .session_store
                    .inc_iteration(&run_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?;

                let messages = self
                    .before_llm_step(&current_messages, &params, context.clone())
                    .await?;
                let step_result = self
                    .llm_step(&messages, &params, context.clone(), event_tx.clone())
                    .await?;

                match step_result {
                    StepResult::Finish(content) => {
                        // Call after_finish hook
                        self.after_finish(&content, context.clone()).await?;

                        tracing::info!("Agent execution completed successfully");
                        let action_step = MemoryStep::Action(ActionStep {
                            model_input_messages: Some(current_messages),
                            model_output: Some(content.clone()),
                            ..Default::default()
                        });
                        self.session_store
                            .store_step(&context.thread_id, action_step.clone())
                            .await
                            .map_err(|e| AgentError::Session(e.to_string()))?;
                        self.logger.log_step(agent_id, &action_step);
                        // Emit text message events if event_tx is provided
                        if let Some(event_tx) = &event_tx {
                            self.emit_text_message_events(
                                event_tx,
                                &context,
                                &content,
                                &Role::Assistant,
                            )
                            .await;
                        }
                        return Ok(content);
                    }
                    StepResult::Continue(new_messages) => {
                        current_messages.extend(new_messages);
                        continue;
                    }
                }
            }
        }
        .await;
        // Send RunFinished or RunError event if event_tx is provided
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
                }
            }
        }
        result
    }

    /// Continue execution with tool responses (for external tool calls)
    pub async fn continue_execution(
        &self,
        tool_response_messages: Vec<Message>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.validate().await?;
        let agent_id = &self.definition.name;
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();
        let mut iterations = self
            .session_store
            .get_iteration(&run_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

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

        let result = async {
            let mut current_messages = self
                .session_store
                .get_messages(&context.thread_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;

            // Add tool response messages to the conversation
            current_messages.extend(tool_response_messages);

            let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS);
            tracing::debug!("Max iterations per run set to: {}", max_iterations);
            
            loop {
                if iterations >= max_iterations {
                    tracing::warn!("Max iterations limit reached: {}", max_iterations);
                    return Err(AgentError::LLMError(format!(
                        "Max iterations reached: {max_iterations}",
                    )));
                }
                iterations = self
                    .session_store
                    .inc_iteration(&run_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?;

                let messages = self
                    .before_llm_step(&current_messages, &None, context.clone())
                    .await?;
                let step_result = self
                    .llm_step(&messages, &None, context.clone(), event_tx.clone())
                    .await?;

                match step_result {
                    StepResult::Finish(content) => {
                        // Call after_finish hook
                        self.after_finish(&content, context.clone()).await?;

                        tracing::info!("Agent execution completed successfully");
                        let action_step = MemoryStep::Action(ActionStep {
                            model_input_messages: Some(current_messages),
                            model_output: Some(content.clone()),
                            ..Default::default()
                        });
                        self.session_store
                            .store_step(&context.thread_id, action_step.clone())
                            .await
                            .map_err(|e| AgentError::Session(e.to_string()))?;
                        self.logger.log_step(agent_id, &action_step);
                        // Emit text message events if event_tx is provided
                        if let Some(event_tx) = &event_tx {
                            self.emit_text_message_events(
                                event_tx,
                                &context,
                                &content,
                                &Role::Assistant,
                            )
                            .await;
                        }
                        return Ok(content);
                    }
                    StepResult::Continue(new_messages) => {
                        current_messages.extend(new_messages);
                        continue;
                    }
                }
            }
        }
        .await;

        // Send RunFinished or RunError event if event_tx is provided
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
                }
            }
        }
        result
    }

    async fn handle_finish_reason(
        &self,
        finish_reason: async_openai::types::FinishReason,
        content: String,
        tool_calls: Vec<ToolCall>,
        agent_id: &str,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<StepResult, AgentError> {
        match finish_reason {
            async_openai::types::FinishReason::Stop => {
                // Return finish result
                Ok(StepResult::Finish(content))
            }
            async_openai::types::FinishReason::ToolCalls => {
                if !tool_calls.is_empty() {
                    // Check if any tool calls are external (frontend tools)
                    let has_external_tools = tool_calls.iter().any(|tc| tc.external);
                    
                    if has_external_tools {
                        // For external tools, we stop execution and return the tool calls
                        // The frontend will handle the execution and call continue_execution
                        let mut new_messages = vec![Message {
                            role: MessageRole::Assistant,
                            name: Some(agent_id.to_string()),
                            content: vec![MessageContent {
                                content_type: "text".to_string(),
                                text: Some(content),
                                image: None,
                            }],
                            tool_calls: tool_calls.to_vec(),
                        }];
                        
                        // Add metadata to indicate these are external tool calls
                        for tool_call in &mut new_messages[0].tool_calls {
                            tool_call.external = true;
                        }
                        
                        return Ok(StepResult::Continue(new_messages));
                    } else {
                        // Process internal tool calls as before
                        let mut new_messages = vec![Message {
                            role: MessageRole::Assistant,
                            name: Some(agent_id.to_string()),
                            content: vec![MessageContent {
                                content_type: "text".to_string(),
                                text: Some(content),
                                image: None,
                            }],
                            tool_calls: tool_calls.to_vec(),
                        }];

                        let tool_calls = self.before_tool_calls(&tool_calls, context.clone()).await?;

                        // Process all tool calls in parallel
                        let tool_responses =
                            futures::future::join_all(tool_calls.iter().map(|mapped_tool_call| {
                                let executor = self.executor.clone();
                                let agent_id = agent_id.to_string();
                                let context = context.clone();
                                let event_tx = event_tx.clone();

                                async move {
                                    let run_id = { context.run_id.lock().await.clone() };
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
                                                AgentError::LLMError(format!(
                                                    "Failed to send ToolCallStart event: {}",
                                                    e
                                                ))
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
                                                AgentError::LLMError(format!(
                                                    "Failed to send ToolCallStart event: {}",
                                                    e
                                                ))
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
                                                AgentError::LLMError(format!(
                                                    "Failed to send ToolCallResult event: {}",
                                                    e
                                                ))
                                            });
                                    }
                                    Message {
                                        role: MessageRole::ToolResponse,
                                        name: Some(mapped_tool_call.tool_name.clone()),
                                        content: vec![MessageContent {
                                            content_type: "text".to_string(),
                                            text: Some(content.clone()),
                                            image: None,
                                        }],
                                        tool_calls: vec![mapped_tool_call.to_owned()],
                                    }
                                }
                            }))
                            .await;

                        let tool_response_contents: Vec<String> = tool_responses
                            .iter()
                            .map(|msg| {
                                msg.content
                                    .first()
                                    .and_then(|c| c.text.clone())
                                    .unwrap_or_default()
                            })
                            .collect();

                        // Call after_tool_calls hook
                        self.after_tool_calls(&tool_response_contents, context)
                            .await?;

                        // Add tool responses
                        new_messages.extend(tool_responses);
                        Ok(StepResult::Continue(new_messages))
                    }
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
impl AgentHooks for StandardAgent {}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum AgentType {
    #[default]
    Standard,
    Remote,
    Custom(String),
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
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        StandardAgent::invoke(self, task, params, context, event_tx).await
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        StandardAgent::invoke_stream(self, task, params, context, event_tx).await
    }

    async fn continue_execution(
        &self,
        tool_response_messages: Vec<Message>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        StandardAgent::continue_execution(self, tool_response_messages, context, event_tx).await
    }
}

// Keep DefaultAgent as an alias for backward compatibility
pub type DefaultAgent = StandardAgent;
