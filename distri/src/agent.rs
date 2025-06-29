use crate::{
    coordinator::{AgentEvent, LocalCoordinator},
    error::AgentError,
    executor::LLMExecutor,
    memory::SystemStep,
    types::{
        get_tool_descriptions, AgentDefinition, Message, MessageContent, MessageRole, PlanConfig,
        ServerTools, DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
    SessionStore,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

use crate::coordinator::{reason::create_initial_plan, CoordinatorContext, StepLogger};
use crate::memory::{ActionStep, MemoryStep, PlanningStep, TaskStep};

pub const MAX_ITERATIONS: i32 = 10;

#[async_trait::async_trait]
pub trait CustomAgent: Send + Sync + std::fmt::Debug {
    /// Main execution step - custom agents implement their logic here
    /// This is called after system prompt, task, and planning (if enabled) have been handled
    /// The context provides access to history, LLM, session writer, and other utilities
    async fn step(&self, context: &mut AgentExecutionContext) -> Result<String, AgentError>;
    
    /// Clone the custom agent (required for object safety)
    fn clone_box(&self) -> Box<dyn CustomAgent>;
}

/// Session writer for custom agents to write steps to memory
pub struct SessionWriter {
    session_store: Arc<Box<dyn SessionStore>>,
    thread_id: String,
    agent_id: String,
}

impl SessionWriter {
    pub fn new(session_store: Arc<Box<dyn SessionStore>>, thread_id: String, agent_id: String) -> Self {
        Self {
            session_store,
            thread_id,
            agent_id,
        }
    }

    /// Write a step to the session memory
    pub async fn write_step(&self, step: MemoryStep) -> Result<(), AgentError> {
        self.session_store
            .store_step(&self.thread_id, step)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    /// Get all messages from the session
    pub async fn get_messages(&self) -> Result<Vec<Message>, AgentError> {
        self.session_store
            .get_messages(&self.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    /// Write a simple text message to the session
    pub async fn write_message(&self, content: &str) -> Result<(), AgentError> {
        let action_step = MemoryStep::Action(ActionStep {
            model_input_messages: None,
            model_output: Some(content.to_string()),
            ..Default::default()
        });
        self.write_step(action_step).await
    }
}

/// LLM executor wrapper for easy access to LLM functions
pub struct LLMExecutorWrapper {
    executor: LLMExecutor,
}

impl LLMExecutorWrapper {
    pub fn new(executor: LLMExecutor) -> Self {
        Self { executor }
    }

    /// Simple LLM call with messages
    pub async fn llm(&self, messages: &[Message], params: Option<serde_json::Value>) -> Result<String, AgentError> {
        let response = self.executor.execute(messages, params).await?;
        Ok(LLMExecutor::extract_first_choice(&response))
    }

    /// Streaming LLM call
    pub async fn llm_stream(&self, messages: &[Message], params: Option<serde_json::Value>, event_tx: mpsc::Sender<AgentEvent>) -> Result<(), AgentError> {
        self.executor.execute_stream(messages, params, event_tx).await
    }
}

/// Execution context provided to custom agents
pub struct AgentExecutionContext {
    pub agent_id: String,
    pub task: TaskStep,
    pub params: Option<serde_json::Value>,
    pub coordinator_context: Arc<CoordinatorContext>,
    pub session_writer: SessionWriter,
    pub llm_executor: LLMExecutorWrapper,
    history: Option<Vec<Message>>,
}

impl AgentExecutionContext {
    pub fn new(
        agent_id: String,
        task: TaskStep,
        params: Option<serde_json::Value>,
        coordinator_context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        executor: LLMExecutor,
    ) -> Self {
        let session_writer = SessionWriter::new(
            session_store,
            coordinator_context.thread_id.clone(),
            agent_id.clone(),
        );
        let llm_executor = LLMExecutorWrapper::new(executor);

        Self {
            agent_id,
            task,
            params,
            coordinator_context,
            session_writer,
            llm_executor,
            history: None,
        }
    }

    /// Load conversation history (lazy loaded)
    pub async fn load_history(&mut self) -> Result<&Vec<Message>, AgentError> {
        if self.history.is_none() {
            self.history = Some(self.session_writer.get_messages().await?);
        }
        Ok(self.history.as_ref().unwrap())
    }

    /// Get the current conversation history (if already loaded)
    pub fn get_history(&self) -> Option<&Vec<Message>> {
        self.history.as_ref()
    }

    /// Refresh the history from the session store
    pub async fn refresh_history(&mut self) -> Result<&Vec<Message>, AgentError> {
        self.history = Some(self.session_writer.get_messages().await?);
        Ok(self.history.as_ref().unwrap())
    }

    /// Simple LLM call using current history
    pub async fn llm_with_history(&mut self, additional_message: Option<&str>) -> Result<String, AgentError> {
        let mut messages = self.load_history().await?.clone();
        
        if let Some(msg) = additional_message {
            messages.push(Message {
                role: MessageRole::User,
                name: Some("user".to_string()),
                content: vec![MessageContent {
                    content_type: "text".to_string(),
                    text: Some(msg.to_string()),
                    image: None,
                }],
                tool_calls: vec![],
            });
        }

        self.llm_executor.llm(&messages, self.params.clone()).await
    }

    /// Simple LLM call with custom messages
    pub async fn llm(&self, messages: &[Message]) -> Result<String, AgentError> {
        self.llm_executor.llm(messages, self.params.clone()).await
    }

    /// Write a message to the session
    pub async fn write_message(&self, content: &str) -> Result<(), AgentError> {
        self.session_writer.write_message(content).await
    }

    /// Log a message (for debugging)
    pub fn log(&self, message: &str) {
        if self.coordinator_context.verbose {
            tracing::info!("[{}] {}", self.agent_id, message);
        }
    }
}

pub struct Agent {
    pub definition: AgentDefinition,
    server_tools: Vec<ServerTools>,
    coordinator: Arc<LocalCoordinator>,
    logger: StepLogger,
    session_store: Arc<Box<dyn SessionStore>>,
    iterations: Arc<RwLock<HashMap<String, i32>>>,
    custom_agent: Option<Box<dyn CustomAgent>>,
}

impl Clone for Agent {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            server_tools: self.server_tools.clone(),
            coordinator: self.coordinator.clone(),
            logger: self.logger.clone(),
            session_store: self.session_store.clone(),
            iterations: self.iterations.clone(),
            custom_agent: self.custom_agent.as_ref().map(|agent| agent.clone_box()),
        }
    }
}
impl Agent {
    pub fn new(
        definition: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Arc<LocalCoordinator>,
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        custom_agent: Option<Box<dyn CustomAgent>>,
    ) -> Self {
        let logger = StepLogger::new(context.verbose);
        Self {
            definition,
            server_tools,
            coordinator,
            logger,
            session_store,
            iterations: Arc::new(RwLock::new(HashMap::new())),
            custom_agent,
        }
    }

    /// Create a local (YAML-based) agent
    pub fn new_local(
        definition: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Arc<LocalCoordinator>,
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self::new(definition, server_tools, coordinator, context, session_store, None)
    }

    /// Create a runnable (CustomAgent-based) agent
    pub fn new_runnable(
        definition: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Arc<LocalCoordinator>,
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        custom_agent: Box<dyn CustomAgent>,
    ) -> Self {
        Self::new(definition, server_tools, coordinator, context, session_store, Some(custom_agent))
    }

    pub async fn plan_step(
        &self,
        task: TaskStep,
        plan_config: &PlanConfig,
        context: Arc<CoordinatorContext>,
    ) -> Result<(), AgentError> {
        let agent_id = &self.definition.name;
        let tools_desc =
            get_tool_descriptions(&self.server_tools, Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE));
        // Get current iteration count
        let iteration = {
            let mut iterations = self.iterations.write().await;
            let count = iterations.entry(agent_id.to_string()).or_insert(0);
            // Update count based on the number of messages for subsequent iterations
            if *count > 0 {
                let previous_messages = self
                    .session_store
                    .get_messages(&context.thread_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?;
                *count = previous_messages.len() as i32; // Set count to number of messages
            } else {
                *count += 1; // Increment for the first iteration
            }
            *count
        };

        if (iteration - 1) % plan_config.interval == 0 {
            // Run either initial planning or planning update
            let (facts, plan) = if iteration == 1 {
                create_initial_plan(&task, &tools_desc, &|msgs| {
                    let planning_executor = LLMExecutor::new(
                        crate::coordinator::reason::get_planning_definition(
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
                crate::coordinator::reason::update_plan(
                    &task.task,
                    &tools_desc,
                    &previous_messages,
                    remaining_steps,
                    &|msgs| {
                        let planning_executor = LLMExecutor::new(
                            crate::coordinator::reason::get_planning_definition(
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

    async fn system_step(&self, context: Arc<CoordinatorContext>) -> Result<(), AgentError> {
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
        context: Arc<CoordinatorContext>,
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

    async fn get_history(
        &self,
        context: Arc<CoordinatorContext>,
    ) -> Result<Vec<Message>, AgentError> {
        self.session_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Get agent definition and tools

        let agent_id = &self.definition.name;

        self.system_step(context.clone()).await?;

        self.task_step(&task, context.clone()).await?;

        let messages = self.get_history(context.clone()).await?;

        // Create executor as a thin wrapper for LLM calls
        let executor = LLMExecutor::new(
            self.definition.clone(),
            self.server_tools.clone(),
            context.clone(),
            None,
            Some(agent_id.to_string()),
        );

        // Execute the streaming LLM call
        executor.execute_stream(&messages, params, event_tx).await
    }

    pub async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        let agent_id = &self.definition.name;
        self.system_step(context.clone()).await?;

        self.task_step(&task, context.clone()).await?;

        // Handle planning if enabled
        if let Some(plan_config) = &self.definition.plan {
            self.plan_step(task.clone(), plan_config, context.clone()).await?;
        }

        // Check if this is a custom agent
        if let Some(custom_agent) = &self.custom_agent {
            // Create executor for the custom agent
            let executor = LLMExecutor::new(
                self.definition.clone(),
                self.server_tools.clone(),
                context.clone(),
                None,
                Some(format!("{}:{}", agent_id, "custom")),
            );

            // Create execution context for the custom agent
            let mut agent_context = AgentExecutionContext::new(
                agent_id.clone(),
                task,
                params,
                context.clone(),
                self.session_store.clone(),
                executor,
            );

            // Execute custom agent's step method
            let result = custom_agent.step(&mut agent_context).await?;

            // Store the custom agent's result as an action step
            let action_step = MemoryStep::Action(ActionStep {
                model_input_messages: agent_context.get_history().cloned(),
                model_output: Some(result.clone()),
                ..Default::default()
            });
            self.session_store
                .store_step(&context.thread_id, action_step.clone())
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &action_step);

            return Ok(result);
        }

        // Default LLM execution for local agents
        // Get all messages from memory steps
        let messages = self
            .session_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Create executor as a thin wrapper for LLM calls
        let executor = LLMExecutor::new(
            self.definition.clone(),
            self.server_tools.clone(),
            context.clone(),
            None,
            Some(format!("{}:{}", agent_id, "execute")),
        );

        // Execute the main execution loop that was previously in AgentExecutor
        let mut token_usage = 0;
        let mut iterations = 0;
        let mut current_messages = messages;

        let max_tokens = self.definition.model_settings.max_tokens;
        let max_iterations = self.definition.model_settings.max_iterations;
        tracing::debug!("Max tokens limit set to: {}", max_tokens);
        tracing::debug!("Max iterations per run set to: {}", max_iterations);

        loop {
            if token_usage > max_tokens {
                tracing::warn!("Max tokens limit reached: {}", max_tokens);
                return Err(AgentError::LLMError(format!(
                    "Max tokens reached: {max_tokens}",
                )));
            }

            if iterations >= max_iterations {
                tracing::warn!("Max iterations limit reached: {}", max_iterations);
                return Err(AgentError::LLMError(format!(
                    "Max iterations reached: {max_iterations}",
                )));
            }
            iterations += 1;

            // Execute a single LLM call through the executor
            let response = executor.execute(&current_messages, params.clone()).await?;

            // Update token usage
            let new_token_usage = response.usage.as_ref().map(|a| a.total_tokens).unwrap_or(0);
            token_usage += new_token_usage;
            tracing::debug!("Current token usage: {}", token_usage);

            // Get the first choice
            let choice = &response.choices[0];
            let finish_reason = choice
                .finish_reason
                .unwrap_or(async_openai::types::FinishReason::Stop);
            let content = choice.message.content.clone().unwrap_or_default();
            let tool_calls = choice.message.tool_calls.clone();

            match finish_reason {
                async_openai::types::FinishReason::Stop => {
                    tracing::info!("Agent execution completed successfully");

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

                    return Ok(content);
                }

                async_openai::types::FinishReason::ToolCalls => {
                    if let Some(tool_calls) = tool_calls {
                        tracing::info!("Processing {} tool calls", tool_calls.len());

                        // Convert assistant message with tool calls
                        let mut new_messages = current_messages.clone();
                        let assistant_message = Message {
                            role: MessageRole::Assistant,
                            name: Some(agent_id.to_string()),
                            content: vec![MessageContent {
                                content_type: "text".to_string(),
                                text: Some(content.clone()),
                                image: None,
                            }],
                            tool_calls: tool_calls.iter().map(LLMExecutor::map_tool_call).collect(),
                        };
                        new_messages.push(assistant_message);

                        // Process all tool calls in parallel
                        let tool_responses = futures::future::join_all(tool_calls.iter().map(
                            |tool_call| async move {
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
                            },
                        ))
                        .await;

                        // Add tool responses to messages
                        new_messages.extend(tool_responses);
                        current_messages = new_messages;
                        continue;
                    }
                }
                x => {
                    tracing::error!("Agent stopped unexpectedly with reason: {:?}", x);
                    return Err(AgentError::LLMError(format!(
                        "Agent stopped with the reason {x:?}"
                    )));
                }
            }
        }
    }
}
