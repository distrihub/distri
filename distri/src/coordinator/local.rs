use crate::{
    error::AgentError,
    executor::LLMExecutor,
    memory::SystemStep,
    servers::registry::ServerRegistry,
    store::{HashMapThreadStore, LocalSessionStore, SessionStore, ThreadStore, ToolSessionStore},
    tools::{execute_tool, get_tools},
    types::{
        get_tool_descriptions, AgentDefinition, CreateThreadRequest, Message, MessageContent,
        MessageRole, ServerTools, Thread, ThreadSummary, UpdateThreadRequest,
        DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

use super::{log::StepLogger, CoordinatorContext};
use super::{reason::create_initial_plan, AgentEvent};
use super::{AgentCoordinator, AgentHandle, CoordinatorMessage};
use crate::memory::{ActionStep, MemoryStep, PlanningStep, TaskStep};

// Message types for coordinator communication

#[derive(Clone)]
pub struct LocalCoordinator {
    pub agent_definitions: Arc<RwLock<HashMap<String, AgentDefinition>>>,
    pub agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
    pub tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub registry: Arc<RwLock<ServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    session_store: Arc<Box<dyn SessionStore>>,
    thread_store: Arc<Box<dyn ThreadStore>>,
    logger: StepLogger,
    iterations: Arc<RwLock<HashMap<String, i32>>>,
    pub context: Arc<CoordinatorContext>,
}

impl LocalCoordinator {
    pub fn new(
        registry: Arc<RwLock<ServerRegistry>>,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
        context: Arc<CoordinatorContext>,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        let thread_store =
            Arc::new(Box::new(HashMapThreadStore::default()) as Box<dyn ThreadStore>);

        let logger = StepLogger::new(context.verbose);
        Self {
            agent_definitions: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
            tool_sessions,
            registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            session_store: session_store
                .unwrap_or_else(|| Arc::new(Box::new(LocalSessionStore::new()))),
            thread_store,
            iterations: Arc::new(RwLock::new(HashMap::new())),
            context: context,
            logger,
        }
    }

    pub fn get_handle(&self, agent_id: String) -> AgentHandle {
        AgentHandle {
            agent_id,
            coordinator_tx: self.coordinator_tx.clone(),
            verbose: self.logger.verbose,
        }
    }

    pub async fn register_agent(&self, definition: AgentDefinition) -> anyhow::Result<()> {
        let (name, resolved_tools) = {
            let name = definition.name.clone();
            let server_tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;

            (name, server_tools)
        };
        // Store both the definition and its tools

        tracing::debug!(
            "Registering agent: {name} with {:?}",
            resolved_tools
                .iter()
                .map(
                    |r| serde_json::json!({"name": r.definition.name, "type": r.definition.r#type, "tools": r.tools.len()})
                )
                .collect::<Vec<_>>()
        );

        {
            let mut definitions = self.agent_definitions.write().await;
            definitions.insert(name.clone(), definition);
        }

        // Store the resolved tools
        let mut tools = self.agent_tools.write().await;
        tools.insert(name, resolved_tools);
        Ok(())
    }

    async fn call_agent_stream(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Get agent definition and tools
        let definition = self.get_agent(agent_id).await?;
        let tools = self.get_tools(agent_id).await?;
        let tools_desc = get_tool_descriptions(&tools, Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE));

        // Store system message if present
        if let Some(system_prompt) = &definition.system_prompt {
            let step = MemoryStep::System(SystemStep {
                system_prompt: system_prompt.clone(),
            });
            self.session_store
                .store_step(&context.thread_id, step.clone())
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &step);
        }

        // Store task step
        let task_step = MemoryStep::Task(task.clone());
        self.session_store
            .store_step(&context.thread_id, task_step.clone())
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        self.logger.log_step(agent_id, &task_step);

        // Handle planning if enabled
        if let Some(planning_config) = &definition.plan {
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

            if (iteration - 1) % planning_config.interval == 0 {
                // Send planning step started event
                let _ = event_tx.send(AgentEvent::StepStarted {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.lock().await.clone(),
                    step_name: "planning".to_string(),
                }).await;
                
                // Run either initial planning or planning update
                let (facts, plan) = if iteration == 1 {
                    create_initial_plan(&task, &tools_desc, &|msgs| {
                        let planning_executor = LLMExecutor::new(
                            super::reason::get_planning_definition(
                                planning_config.model_settings.clone(),
                            ),
                            vec![],
                            context.clone(),
                            None,
                            Some(format!("{}:{}", agent_id, "initial_plan")),
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
                    let remaining_steps =
                        planning_config.max_iterations.unwrap_or(10) - iteration + 1;
                    let previous_messages = self
                        .session_store
                        .get_messages(&context.thread_id)
                        .await
                        .map_err(|e| AgentError::Session(e.to_string()))?;
                    super::reason::update_plan(
                        &task.task,
                        &tools_desc,
                        &previous_messages,
                        remaining_steps,
                        &|msgs| {
                            let planning_executor = LLMExecutor::new(
                                super::reason::get_planning_definition(
                                    planning_config.model_settings.clone(),
                                ),
                                vec![],
                                context.clone(),
                                None,
                                Some(format!("{}:{}", agent_id, "update_plan")),
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
                
                // Send planning step finished event
                let _ = event_tx.send(AgentEvent::StepFinished {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.lock().await.clone(),
                    step_name: "planning".to_string(),
                }).await;
            }
        }

        // Get all messages from memory steps
        let messages = self
            .session_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Create executor as a thin wrapper for LLM calls
        let executor = LLMExecutor::new(
            definition.clone(),
            tools,
            context.clone(),
            None,
            Some(agent_id.to_string()),
        );

        // Execute the streaming LLM call
        executor.execute_stream(&messages, params, event_tx).await
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        tracing::info!("AgentCoordinator run loop started");

        while let Some(msg) = self.coordinator_rx.lock().await.recv().await {
            tracing::info!("AgentCoordinator received a message: {:?}", msg);
            match msg {
                CoordinatorMessage::ExecuteTool {
                    agent_id,
                    tool_call,
                    response_tx,
                } => {
                    tracing::info!("Handling ExecuteTool for agent: {}", agent_id);
                    let registry = self.registry.clone();
                    let agent_tools = self.agent_tools.clone();
                    let tool_sessions = self.tool_sessions.clone();
                    let context = self.context.clone();
                    tokio::spawn(async move {
                        let result = async {
                            // Get the server tools in a separate scope to release the lock quickly
                            let server_tools = {
                                let tools = agent_tools.read().await;
                                tools.get(&agent_id).cloned()
                            };

                            match server_tools {
                                Some(server_tools) => {
                                    // Execute the tool
                                    let tool_def = server_tools.iter().find(|t| {
                                        t.tools.iter().any(|tool| tool.name == tool_call.tool_name)
                                    });

                                    let content = match tool_def {
                                        Some(server_tool) => execute_tool(
                                            &tool_call,
                                            &server_tool.definition,
                                            registry,
                                            tool_sessions,
                                            context,
                                        )
                                        .await
                                        .unwrap_or_else(|err| format!("Error: {}", err)),
                                        None => format!("Tool not found {}", tool_call.tool_name),
                                    };
                                    Ok(content)
                                }
                                None => Err(AgentError::ToolExecution(format!(
                                    "Agent {} not found",
                                    agent_id
                                ))),
                            }
                        }
                        .await;

                        let _ =
                            response_tx.send(result.unwrap_or_else(|e| format!("Error: {}", e)));
                    });
                }
                CoordinatorMessage::Execute {
                    agent_id,
                    task,
                    params,
                    context,
                    response_tx,
                } => {
                    tracing::info!(
                        "Handling Execute for agent: {} with messages: {:?}",
                        agent_id,
                        task
                    );
                    let this = self.clone();
                    tokio::spawn(async move {
                        let result =
                            async { this.call_agent(&agent_id, task, params, context).await }.await;

                        let _ = response_tx.send(result);
                    });
                }
                CoordinatorMessage::ExecuteStream {
                    agent_id,
                    task,
                    params,
                    event_tx,
                    context,
                } => {
                    tracing::info!(
                        "Handling ExecuteStream for agent: {} with messages: {:?}",
                        agent_id,
                        task
                    );
                    let this = self.clone();
                    tokio::spawn(async move {
                        let result = async {
                            this.call_agent_stream(&agent_id, task, params, context, event_tx)
                                .await
                        }
                        .await;

                        if let Err(e) = result {
                            tracing::error!("Error in streaming execution: {}", e);
                        }
                    });
                }
            }
        }
        tracing::info!("AgentCoordinator run loop exiting");
        Ok(())
    }

    async fn call_agent(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        // Get agent definition and tools
        let definition = self.get_agent(agent_id).await?;
        let tools = self.get_tools(agent_id).await?;
        let tools_desc = get_tool_descriptions(&tools, Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE));
        // Store system message if present
        if let Some(system_prompt) = &definition.system_prompt {
            let step = MemoryStep::System(SystemStep {
                system_prompt: system_prompt.clone(),
            });
            self.session_store
                .store_step(&context.thread_id, step.clone())
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &step);
        }

        // Store task step
        let task_step = MemoryStep::Task(task.clone());
        self.session_store
            .store_step(&context.thread_id, task_step.clone())
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        self.logger.log_step(agent_id, &task_step);

        // Handle planning if enabled
        if let Some(planning_config) = &definition.plan {
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

            if (iteration - 1) % planning_config.interval == 0 {
                // Run either initial planning or planning update
                let (facts, plan) = if iteration == 1 {
                    create_initial_plan(&task, &tools_desc, &|msgs| {
                        let planning_executor = LLMExecutor::new(
                            super::reason::get_planning_definition(
                                planning_config.model_settings.clone(),
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
                    let remaining_steps =
                        planning_config.max_iterations.unwrap_or(10) - iteration + 1;
                    let previous_messages = self
                        .session_store
                        .get_messages(&context.thread_id)
                        .await
                        .map_err(|e| AgentError::Session(e.to_string()))?;
                    super::reason::update_plan(
                        &task.task,
                        &tools_desc,
                        &previous_messages,
                        remaining_steps,
                        &|msgs| {
                            let planning_executor = LLMExecutor::new(
                                super::reason::get_planning_definition(
                                    planning_config.model_settings.clone(),
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
        }

        // Get all messages from memory steps
        let messages = self
            .session_store
            .get_messages(&context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Create executor as a thin wrapper for LLM calls
        let executor = LLMExecutor::new(
            definition.clone(),
            tools,
            context.clone(),
            None,
            Some(format!("{}:{}", agent_id, "execute")),
        );

        // Execute the main execution loop that was previously in AgentExecutor
        let mut token_usage = 0;
        let mut iterations = 0;
        let mut current_messages = messages;
        let handle = self.get_handle(agent_id.to_string());

        let max_tokens = definition.model_settings.max_tokens;
        let max_iterations = definition.model_settings.max_iterations;
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
                        let tool_responses =
                            futures::future::join_all(tool_calls.iter().map(|tool_call| {
                                let handle = handle.clone();
                                async move {
                                    let mapped_tool_call = LLMExecutor::map_tool_call(tool_call);

                                    let content = handle
                                        .execute_tool(mapped_tool_call.clone())
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
                                }
                            }))
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

    // Thread management methods
    pub async fn create_thread(&self, request: CreateThreadRequest) -> Result<Thread, AgentError> {
        // Validate that the agent exists
        let agent_definitions = self.agent_definitions.read().await;
        if !agent_definitions.contains_key(&request.agent_id) {
            return Err(AgentError::NotFound(format!(
                "Agent '{}' not found",
                request.agent_id
            )));
        }
        drop(agent_definitions);

        self.thread_store
            .create_thread(request)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn get_thread(&self, thread_id: &str) -> Result<Option<Thread>, AgentError> {
        self.thread_store
            .get_thread(thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> Result<Thread, AgentError> {
        self.thread_store
            .update_thread(thread_id, request)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn delete_thread(&self, thread_id: &str) -> Result<(), AgentError> {
        self.thread_store
            .delete_thread(thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn list_threads(
        &self,
        agent_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ThreadSummary>, AgentError> {
        self.thread_store
            .list_threads(agent_id, limit, offset)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    /// Ensures a thread exists for the given agent and thread_id, creating it if necessary.
    /// Optionally takes an initial message for thread creation.
    pub async fn ensure_thread_exists(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
        initial_message: Option<String>,
    ) -> Result<Thread, AgentError> {
        let thread = match thread_id {
            Some(thread_id) => self.get_thread(thread_id).await?,
            None => None,
        };
        match thread {
            Some(thread) => Ok(thread),
            None => {
                let create_request = crate::types::CreateThreadRequest {
                    agent_id: agent_id.to_string(),
                    title: None,
                    initial_message,
                };
                self.create_thread(create_request).await
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentCoordinator for LocalCoordinator {
    async fn list_agents(
        &self,
        _cursor: Option<String>,
    ) -> Result<(Vec<AgentDefinition>, Option<String>), AgentError> {
        let definitions = self.agent_definitions.read().await;
        let agents: Vec<AgentDefinition> = definitions.values().take(30).cloned().collect();
        Ok((agents, None))
    }

    async fn get_agent(&self, agent_name: &str) -> Result<AgentDefinition, AgentError> {
        let definitions = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.agent_definitions.read(),
        )
        .await
        .map_err(|_| AgentError::ToolExecution("get_agent timed out".into()))?;

        definitions
            .get(agent_name)
            .cloned()
            .ok_or_else(|| AgentError::ToolExecution(format!("Agent {} not found", agent_name)))
    }

    async fn get_tools(&self, agent_name: &str) -> Result<Vec<ServerTools>, AgentError> {
        let tools =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.agent_tools.read())
                .await
                .map_err(|_| AgentError::ToolExecution("get_tools timed out".into()))?;

        Ok(tools.get(agent_name).cloned().unwrap_or_default())
    }

    async fn execute(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        let result = self.call_agent(agent_name, task, params, context).await?;

        // Update thread store with agent definitions for thread listing
        let agent_definitions = self.agent_definitions.read().await;
        if let Some(thread_store) = self
            .thread_store
            .as_ref()
            .as_any()
            .downcast_ref::<HashMapThreadStore>()
        {
            thread_store
                .set_agent_definitions(agent_definitions.clone())
                .await;
        }

        Ok(result)
    }

    async fn execute_stream(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: mpsc::Sender<AgentEvent>,
        context: Arc<CoordinatorContext>,
    ) -> Result<(), AgentError> {
        self.call_agent_stream(agent_name, task, params, context, event_tx)
            .await?;

        // Update thread store with agent definitions for thread listing
        let agent_definitions = self.agent_definitions.read().await;
        if let Some(thread_store) = self
            .thread_store
            .as_ref()
            .as_any()
            .downcast_ref::<HashMapThreadStore>()
        {
            thread_store
                .set_agent_definitions(agent_definitions.clone())
                .await;
        }

        Ok(())
    }
}
