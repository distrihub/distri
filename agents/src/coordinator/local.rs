use crate::{
    error::AgentError,
    executor::AgentExecutor,
    servers::{
        memory::{ActionStep, AgentMemory},
        registry::ServerRegistry,
    },
    store::ToolSessionStore,
    tools::{execute_tool, get_tools},
    types::{
        get_tool_descriptions, AgentDefinition, Message, MessageContent, MessageRole, ServerTools,
        DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

use super::log::StepLogger;
use super::reason::create_initial_plan;
use super::{AgentCoordinator, AgentHandle, CoordinatorMessage};
use crate::servers::memory::{LocalAgentMemory, MemoryStep, PlanningStep, TaskStep};

// Define trait for memory storage
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn get_messages(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<Message>> {
        let steps = self.get_steps(agent_id, thread_id).await?;
        let messages = steps
            .iter()
            .flat_map(|step| step.to_messages(false, true))
            .collect();
        Ok(messages)
    }
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>>;
    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()>;
}

// Local implementation using HashMap
#[derive(Clone)]
pub struct LocalMemoryStore {
    memories: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
}

impl LocalMemoryStore {
    pub fn new() -> Self {
        Self {
            memories: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MemoryStore for LocalMemoryStore {
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>> {
        let memories = self.memories.read().await;
        let memory = memories
            .get(agent_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::new);
        Ok(memory.get_steps(thread_id))
    }

    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        let memory = memories
            .entry(agent_id.to_string())
            .or_insert_with(LocalAgentMemory::new);
        memory.add_step(step, thread_id);
        Ok(())
    }
}

// Message types for coordinator communication

#[derive(Clone)]
pub struct LocalCoordinator {
    agent_definitions: Arc<RwLock<HashMap<String, AgentDefinition>>>,
    agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    registry: Arc<RwLock<ServerRegistry>>,
    coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    memory_store: Arc<Box<dyn MemoryStore>>,
    logger: StepLogger,
}

impl LocalCoordinator {
    pub fn new(
        registry: Arc<RwLock<ServerRegistry>>,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
        memory_store: Option<Arc<Box<dyn MemoryStore>>>,
        verbose: bool,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        Self {
            agent_definitions: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
            tool_sessions,
            registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            memory_store: memory_store
                .unwrap_or_else(|| Arc::new(Box::new(LocalMemoryStore::new()))),
            logger: StepLogger::new(verbose),
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
                            async { this.run_task(&agent_id, task, None, params).await }.await;

                        let _ = response_tx.send(result);
                    });
                }
            }
        }
        tracing::info!("AgentCoordinator run loop exiting");
        Ok(())
    }

    async fn run_task(
        &self,
        agent_id: &str,
        task: TaskStep,
        thread_id: Option<&str>,
        params: Option<serde_json::Value>,
    ) -> Result<String, AgentError> {
        // Get agent definition and tools
        let definition = self.get_agent(agent_id).await?;
        let tools = self.get_tools(agent_id).await?;
        let tools_desc = get_tool_descriptions(&tools, Some(DEFAULT_TOOL_DESCRIPTION_TEMPLATE));
        // Store system message if present
        if let Some(system_prompt) = &definition.system_prompt {
            let step = MemoryStep::Task(TaskStep {
                task: system_prompt.clone(),
                task_images: None,
            });
            self.memory_store
                .store_step(agent_id, step.clone(), thread_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
            self.logger.log_step(agent_id, &step);
        }

        // Store task step
        let task_step = MemoryStep::Task(task.clone());
        self.memory_store
            .store_step(agent_id, task_step.clone(), thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        self.logger.log_step(agent_id, &task_step);

        // Create planning if enabled in agent definition
        if let Some(planning_config) = &definition.planning_config {
            if planning_config.enabled {
                let (facts, plan) = create_initial_plan(&task, &tools_desc, &|msgs| {
                    let planning_executor = AgentExecutor::new(
                        super::reason::get_planning_definition(),
                        vec![],
                        Some(Arc::new(self.get_handle(agent_id.to_string()))),
                        self.logger.verbose,
                    );
                    Box::pin(async move {
                        planning_executor
                            .execute(&msgs, None)
                            .await
                            .map_err(|e| anyhow::anyhow!("Planning execution failed: {}", e))
                    })
                })
                .await
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
                    },
                    plan: plan.clone(),
                });
                self.memory_store
                    .store_step(agent_id, planning_step.clone(), thread_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?;
                self.logger.log_step(agent_id, &planning_step);
            }
        }

        // Get all messages from memory steps
        let messages = self
            .memory_store
            .get_messages(agent_id, thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Execute with agent executor
        let executor = AgentExecutor::new(
            definition,
            tools,
            Some(Arc::new(self.get_handle(agent_id.to_string()))),
            self.logger.verbose,
        );

        let response = executor
            .execute(&messages, params)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Store response as action step
        let action_step = MemoryStep::Action(ActionStep {
            model_input_messages: Some(messages),
            model_output: Some(response.clone()),
            ..Default::default()
        });
        self.memory_store
            .store_step(agent_id, action_step.clone(), thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        self.logger.log_step(agent_id, &action_step);

        Ok(response)
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
    ) -> Result<String, AgentError> {
        self.run_task(agent_name, task, None, params).await
    }
}
