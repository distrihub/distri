use crate::{
    agent::{AgentEvent, AgentExecutor},
    error::AgentError,
    llm::LLMExecutor,
    memory::SystemStep,
    types::{
        get_tool_descriptions, AgentDefinition, Message, MessageContent, MessageRole, PlanConfig,
        ServerTools, ToolCall, DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
    SessionStore,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::agent::{reason::create_initial_plan, ExecutorContext, StepLogger};
use crate::memory::{ActionStep, MemoryStep, PlanningStep, TaskStep};

pub const MAX_ITERATIONS: i32 = 10;

#[async_trait::async_trait]
pub trait BaseAgent: Send + Sync + std::fmt::Debug {
    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError>;
    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError>;

    // Default implementation hooks that return values as-is
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn after_llm_step(
        &self,
        messages: &[Message],
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        Ok(messages.to_vec())
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[ToolCall],
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<ToolCall>, AgentError> {
        Ok(tool_calls.to_vec())
    }

    async fn after_tool_calls(
        &self,
        _tool_responses: &[String],
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn after_finish(
        &self,
        _content: &str,
        _context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    /// Clone the agent (required for object safety)
    fn clone_box(&self) -> Box<dyn BaseAgent>;

    /// Get the agent's name/id
    fn get_name(&self) -> &str;

    fn get_description(&self) -> &str;
    fn get_definition(&self) -> AgentDefinition;
}

#[async_trait::async_trait]
pub trait AgentInvoke: BaseAgent {
    /// Custom invoke implementation
    /// By default this errors out - implementers must override this
    async fn agent_invoke(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        Err(AgentError::NotImplemented(
            "AgentInvoke::agent_invoke not implemented".to_string(),
        ))
    }
}

#[async_trait::async_trait]
pub trait AgentInvokeStream: BaseAgent {
    /// Custom invoke_stream implementation
    /// By default this errors out - implementers must override this
    async fn agent_invoke_stream(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        Err(AgentError::NotImplemented(
            "AgentInvokeStream::agent_invoke_stream not implemented".to_string(),
        ))
    }
}

/// Result of a single step execution
#[derive(Debug)]
pub enum StepResult {
    /// Continue with more iterations, with new messages to add
    Continue(Vec<Message>),
    /// Finish execution with this final response
    Finish(String),
    /// Handle tool calls (for custom agents that want to manage tools)
    ToolCalls(Vec<crate::types::ToolCall>),
}

/// Default agent implementation that provides the standard LLM-based behavior
#[derive(Clone)]
pub struct DefaultAgent {
    pub definition: AgentDefinition,
    server_tools: Vec<ServerTools>,
    coordinator: Arc<AgentExecutor>,
    logger: StepLogger,
    session_store: Arc<Box<dyn SessionStore>>,
    iterations: Arc<RwLock<HashMap<String, i32>>>,
}

impl std::fmt::Debug for DefaultAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultAgent")
            .field("definition", &self.definition)
            .finish()
    }
}

impl DefaultAgent {
    pub fn new(
        definition: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let logger = StepLogger::new(context.verbose);
        Self {
            definition,
            server_tools,
            coordinator,
            logger,
            session_store,
            iterations: Arc::new(RwLock::new(HashMap::new())),
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
        let tools_desc =
            get_tool_descriptions(&self.server_tools, Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE));

        if (iteration - 1) % plan_config.interval == 0 {
            // Run either initial planning or planning update
            let (facts, plan) = if iteration == 1 {
                create_initial_plan(&task, &tools_desc, &|msgs| {
                    let planning_executor = LLMExecutor::new(
                        crate::agent::reason::get_planning_definition(
                            plan_config.model_settings.clone(),
                        ),
                        vec![],
                        context.clone(),
                        None,
                        Some("initial_plan".to_string()),
                    );
                    Box::pin(async move {
                        let response = planning_executor.execute(&msgs, None).await;
                        match response {
                            Ok(response) => {
                                // Extract just the content string
                                let content = LLMExecutor::extract_first_choice(&response);
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
                            vec![],
                            context.clone(),
                            None,
                            Some("update_plan".to_string()),
                        );
                        Box::pin(async move {
                            let response = planning_executor.execute(&msgs, None).await;
                            match response {
                                Ok(response) => {
                                    // Extract just the content string
                                    let content = LLMExecutor::extract_first_choice(&response);
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
    /// Execute one step in the execution loop
    /// Executes standard LLM call with tool handling
    async fn step(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        self.llm_step(messages, params, context).await
    }

    /// Execute one step using LLM
    async fn llm_step(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
    ) -> Result<StepResult, AgentError> {
        let agent_id = &self.definition.name;

        // Create executor for LLM call
        let executor = LLMExecutor::new(
            self.definition.clone(),
            self.server_tools.clone(),
            context.clone(),
            None,
            Some(format!("{}:{}", agent_id, "step")),
        );

        // Execute LLM call
        let response = executor.execute(messages, params).await?;

        // Get the first choice
        let choice = &response.choices[0];
        let finish_reason = choice
            .finish_reason
            .unwrap_or(async_openai::types::FinishReason::Stop);
        let content = choice.message.content.clone().unwrap_or_default();
        let tool_calls = choice.message.tool_calls.clone();

        match finish_reason {
            async_openai::types::FinishReason::Stop => {
                // Return finish result
                Ok(StepResult::Finish(content))
            }
            async_openai::types::FinishReason::ToolCalls => {
                if let Some(tool_calls) = tool_calls {
                    // Convert and add assistant message with tool calls
                    let mut new_messages = vec![Message {
                        role: MessageRole::Assistant,
                        name: Some(agent_id.to_string()),
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(content),
                            image: None,
                        }],
                        tool_calls: tool_calls.iter().map(LLMExecutor::map_tool_call).collect(),
                    }];

                    // Process all tool calls in parallel
                    let tool_responses =
                        futures::future::join_all(tool_calls.iter().map(|tool_call| async move {
                            let mapped_tool_call = LLMExecutor::map_tool_call(tool_call);

                            let content = self
                                .coordinator
                                .execute_tool(agent_id.clone(), mapped_tool_call.clone())
                                .await
                                .unwrap_or_else(|err| format!("Error: {}", err));

                            Message {
                                role: MessageRole::ToolResponse,
                                name: Some(tool_call.function.name.clone()),
                                content: vec![MessageContent {
                                    content_type: "text".to_string(),
                                    text: Some(content),
                                    image: None,
                                }],
                                tool_calls: vec![mapped_tool_call],
                            }
                        }))
                        .await;

                    // Add tool responses
                    new_messages.extend(tool_responses);
                    Ok(StepResult::Continue(new_messages))
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

    /// Helper to emit TextMessage* events for a message
    async fn emit_text_message_events(
        &self,
        event_tx: &mpsc::Sender<AgentEvent>,
        context: &ExecutorContext,
        content: &str,
        role: &str,
    ) {
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();
        let message_id = Uuid::new_v4().to_string();
        let _ = event_tx
            .send(AgentEvent::TextMessageStart {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                message_id: message_id.clone(),
                role: role.to_string(),
            })
            .await;
        let _ = event_tx
            .send(AgentEvent::TextMessageContent {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                message_id: message_id.clone(),
                delta: content.to_string(),
            })
            .await;
        let _ = event_tx
            .send(AgentEvent::TextMessageEnd {
                thread_id,
                run_id,
                message_id,
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
            .send(AgentEvent::RunStarted {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
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
                // Decide if this step should be streaming (LLM step) or not
                let is_llm_step = true; // For now, always stream LLM step
                if is_llm_step {
                    let executor = LLMExecutor::new(
                        self.definition.clone(),
                        self.server_tools.clone(),
                        context.clone(),
                        None,
                        Some(agent_id.to_string()),
                    );
                    // Streaming LLM step: propagate deltas via event_tx
                    executor
                        .execute_stream(&current_messages, params.clone(), event_tx.clone())
                        .await?;
                    // After streaming, break (one streaming step per invoke_stream)
                    break Ok(());
                } else {
                    // Non-streaming step (e.g., tool calls)
                    let step_result = self
                        .step(&current_messages, params.clone(), context.clone())
                        .await?;
                    match step_result {
                        StepResult::Finish(content) => {
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
                            // Emit text message events
                            self.emit_text_message_events(
                                &event_tx,
                                &context,
                                &content,
                                "assistant",
                            )
                            .await;
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
                        StepResult::ToolCalls(_tool_calls) => {
                            return Err(AgentError::LLMError(
                                "ToolCalls result not properly handled".to_string(),
                            ));
                        }
                    }
                }
            }
        }
        .await;

        // Send RunFinished or RunError event
        match &result {
            Ok(_) => {
                let _ = event_tx
                    .send(AgentEvent::RunFinished { thread_id, run_id })
                    .await;
            }
            Err(e) => {
                let _ = event_tx
                    .send(AgentEvent::RunError {
                        thread_id,
                        run_id,
                        message: e.to_string(),
                        code: None,
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
                .send(AgentEvent::RunStarted {
                    thread_id: thread_id.clone(),
                    run_id: run_id.clone(),
                })
                .await;
        }
        let result = async {
            if iterations == 0 {
                self.system_step(context.clone()).await?;
                self.task_step(&task, context.clone()).await?;
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
                let step_result = self
                    .step(&current_messages, params.clone(), context.clone())
                    .await?;
                match step_result {
                    StepResult::Finish(content) => {
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
                                "assistant",
                            )
                            .await;
                        }
                        return Ok(content);
                    }
                    StepResult::Continue(new_messages) => {
                        current_messages.extend(new_messages);
                        continue;
                    }
                    StepResult::ToolCalls(_tool_calls) => {
                        return Err(AgentError::LLMError(
                            "ToolCalls result not properly handled".to_string(),
                        ));
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
                        .send(AgentEvent::RunFinished { thread_id, run_id })
                        .await;
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AgentEvent::RunError {
                            thread_id,
                            run_id,
                            message: e.to_string(),
                            code: None,
                        })
                        .await;
                }
            }
        }
        result
    }
}

#[async_trait::async_trait]
impl BaseAgent for DefaultAgent {
    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
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
                .send(AgentEvent::RunStarted {
                    thread_id: thread_id.clone(),
                    run_id: run_id.clone(),
                })
                .await;
        }
        let result = async {
            if iterations == 0 {
                self.system_step(context.clone()).await?;
                self.task_step(&task, context.clone()).await?;
            }
            let mut current_messages = self
                .session_store
                .get_messages(&context.thread_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            let max_iterations = self.definition.max_iterations.unwrap_or(MAX_ITERATIONS);
            tracing::debug!("Max iterations per run set to: {}", max_iterations);
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
                    // Refresh messages after planning
                    current_messages = self
                        .session_store
                        .get_messages(&context.thread_id)
                        .await
                        .map_err(|e| AgentError::Session(e.to_string()))?;
                }
                let step_result = self
                    .step(&current_messages, params.clone(), context.clone())
                    .await?;
                match step_result {
                    StepResult::Finish(content) => {
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
                        // Call after_finish hook
                        self.after_finish(&content, context.clone()).await?;
                        return Ok(content);
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
                    StepResult::ToolCalls(_tool_calls) => {
                        return Err(AgentError::LLMError(
                            "ToolCalls result not properly handled".to_string(),
                        ));
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
                        .send(AgentEvent::RunFinished { thread_id, run_id })
                        .await;
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AgentEvent::RunError {
                            thread_id,
                            run_id,
                            message: e.to_string(),
                            code: None,
                        })
                        .await;
                }
            }
        }
        result
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Use the actual implementation from DefaultAgent
        DefaultAgent::invoke_stream(self, task, params, context, event_tx).await
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }
}

/// Example custom agent implementation that uses AgentInvoke
#[derive(Debug, Clone)]
pub struct TestCustomAgent {
    pub name: String,
    pub description: String,
}

impl TestCustomAgent {
    pub fn new(name: String) -> Self {
        Self {
            description: name.clone(),
            name: name.clone(),
        }
    }
}

#[async_trait::async_trait]
impl BaseAgent for TestCustomAgent {
    fn get_description(&self) -> &str {
        &self.description
    }

    fn get_definition(&self) -> AgentDefinition {
        AgentDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            ..Default::default()
        }
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        // Call the custom agent_invoke implementation
        self.agent_invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Call the custom agent_invoke_stream implementation
        self.agent_invoke_stream(task, params, context, event_tx)
            .await
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[async_trait::async_trait]
impl AgentInvoke for TestCustomAgent {
    async fn agent_invoke(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();

        // Send RunStarted event if event_tx is provided
        if let Some(event_tx) = &event_tx {
            let _ = event_tx
                .send(AgentEvent::RunStarted {
                    thread_id: thread_id.clone(),
                    run_id: run_id.clone(),
                })
                .await;
        }

        // Custom processing: just return a simple response based on the task
        let response = format!("Custom agent '{}' processed task: {}", self.name, task.task);

        // Send RunFinished event if event_tx is provided
        if let Some(event_tx) = &event_tx {
            let _ = event_tx
                .send(AgentEvent::RunFinished { thread_id, run_id })
                .await;
        }

        Ok(response)
    }
}

#[async_trait::async_trait]
impl AgentInvokeStream for TestCustomAgent {
    async fn agent_invoke_stream(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        let run_id = context.run_id.lock().await.clone();
        let thread_id = context.thread_id.clone();

        // Send RunStarted event
        let _ = event_tx
            .send(AgentEvent::RunStarted {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
            })
            .await;

        // Simulate streaming response by sending multiple text events
        let message_id = uuid::Uuid::new_v4().to_string();
        let _ = event_tx
            .send(AgentEvent::TextMessageStart {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                message_id: message_id.clone(),
                role: "assistant".to_string(),
            })
            .await;

        let str = &format!("'{}' ", self.get_name());
        let parts = vec![
            "Custom ",
            "agent ",
            str,
            "is ",
            "streaming ",
            "response ",
            "for: ",
            &task.task,
        ];

        for part in parts {
            let _ = event_tx
                .send(AgentEvent::TextMessageContent {
                    thread_id: thread_id.clone(),
                    run_id: run_id.clone(),
                    message_id: message_id.clone(),
                    delta: part.to_string(),
                })
                .await;

            // Small delay to simulate streaming
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        let _ = event_tx
            .send(AgentEvent::TextMessageEnd {
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
                message_id,
            })
            .await;

        // Send RunFinished event
        let _ = event_tx
            .send(AgentEvent::RunFinished { thread_id, run_id })
            .await;

        Ok(())
    }
}
