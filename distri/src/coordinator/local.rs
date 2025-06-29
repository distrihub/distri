use crate::{
    agent::Agent,
    error::AgentError,
    executor::LLMExecutor,
    memory::SystemStep,
    servers::registry::ServerRegistry,
    store::{
        AgentStore, HashMapThreadStore, LocalSessionStore, SessionStore, ThreadStore,
        ToolSessionStore,
    },
    tools::{execute_tool, get_tools},
    types::{
        get_tool_descriptions, AgentDefinition, AgentRecord, CreateThreadRequest, Message,
        MessageContent, MessageRole, ServerTools, Thread, ThreadSummary, ToolCall,
        UpdateThreadRequest, DEFAULT_TOOL_DESCRIPTION_TEMPLATE,
    },
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use super::{log::StepLogger, CoordinatorContext};
use super::{reason::create_initial_plan, AgentEvent};
use super::{AgentCoordinator, AgentHandle, CoordinatorMessage};
use crate::memory::{ActionStep, MemoryStep, PlanningStep, TaskStep};

// Message types for coordinator communication

#[derive(Clone)]
pub struct LocalCoordinator {
    pub agent_store: Arc<Box<dyn AgentStore>>,
    pub tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub registry: Arc<RwLock<ServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub session_store: Arc<Box<dyn SessionStore>>,
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
        agent_store: Arc<Box<dyn AgentStore>>,
        context: Arc<CoordinatorContext>,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        let thread_store =
            Arc::new(Box::new(HashMapThreadStore::default()) as Box<dyn ThreadStore>);

        let logger = StepLogger::new(context.verbose);
        Self {
            agent_store: agent_store,
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

    pub async fn execute_tool(
        &self,
        agent_id: String,
        tool_call: ToolCall,
    ) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteTool {
                agent_id: agent_id.clone(),
                tool_call,
                response_tx,
            })
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!("Failed to send tool execution request: {}", e))
            })?;

        response_rx.await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to receive tool response: {}", e))
        })
    }

    pub async fn register_agent(&self, record: AgentRecord) -> anyhow::Result<Agent> {
        let (definition, agent) = match record.clone() {
            AgentRecord::Local(definition) => (
                definition.clone(),
                Agent::new_local(
                    definition,
                    vec![],
                    Arc::new(self.clone()),
                    self.context.clone(),
                    self.session_store.clone(),
                ),
            ),

            AgentRecord::Runnable(definition, custom_agent) => (
                definition.clone(),
                Agent::new_runnable(
                    definition,
                    vec![],
                    Arc::new(self.clone()),
                    self.context.clone(),
                    self.session_store.clone(),
                    custom_agent,
                ),
            ),
        };

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

        self.agent_store
            .register(agent.clone(), resolved_tools)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(agent)
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
                    let agent_store = self.agent_store.clone();
                    let tool_sessions = self.tool_sessions.clone();
                    let context = self.context.clone();
                    tokio::spawn(async move {
                        let result = async {
                            // Get the server tools for the agent
                            let server_tools = agent_store.get_tools(&agent_id).await;

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
        // Get agent from store and use invoke method
        if let Some(agent) = self.agent_store.get(agent_id).await {
            agent.invoke(task, params, context).await
        } else {
            Err(AgentError::NotFound(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    async fn call_agent_stream(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Get agent from store and use invoke_stream method
        if let Some(agent) = self.agent_store.get(agent_id).await {
            agent.invoke_stream(task, params, context, event_tx).await
        } else {
            Err(AgentError::NotFound(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    // Thread management methods
    pub async fn create_thread(&self, request: CreateThreadRequest) -> Result<Thread, AgentError> {
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
        thread_id: Option<String>,
        initial_message: Option<String>,
    ) -> Result<Thread, AgentError> {
        let thread = match &thread_id {
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
                    thread_id,
                };
                self.create_thread(create_request).await
            }
        }
    }
}

#[async_trait::async_trait]
impl AgentCoordinator for LocalCoordinator {
    async fn execute(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        self.call_agent(agent_name, task, params, context).await
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
            .await
    }
}
