use crate::{
    agent::{BaseAgent, StandardAgent},
    error::AgentError,
    servers::registry::{register_servers, ServerMetadata, ServerRegistry},
    store::{
        AgentStore, HashMapThreadStore, LocalSessionStore, SessionStore, ThreadStore,
        ToolSessionStore,
    },
    stores::HashMapTaskStore,
    tools::{execute_tool, get_tools},
    types::{
        Configuration, CreateThreadRequest, Thread, ThreadSummary, ToolCall, UpdateThreadRequest,
    },
    TaskStore,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use super::AgentEvent;
use super::CoordinatorMessage;
use super::ExecutorContext;
use crate::memory::TaskStep;

// Message types for coordinator communication

#[allow(dead_code)]
#[derive(Clone)]
pub struct AgentExecutor {
    pub agent_store: Arc<dyn AgentStore>,
    pub tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub registry: Arc<RwLock<ServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub session_store: Arc<Box<dyn SessionStore>>,
    thread_store: Arc<Box<dyn ThreadStore>>,
    task_store: Arc<dyn TaskStore>,
    pub context: Arc<ExecutorContext>,
}

pub struct AgentExecutorBuilder {
    registry: Option<Arc<RwLock<ServerRegistry>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
    agent_store: Option<Arc<dyn AgentStore>>,
    task_store: Option<Arc<dyn TaskStore>>,
    thread_store: Option<Arc<Box<dyn ThreadStore>>>,
    context: Option<Arc<ExecutorContext>>,
}

impl AgentExecutorBuilder {
    pub fn new() -> Self {
        Self {
            registry: None,
            tool_sessions: None,
            session_store: None,
            agent_store: None,
            task_store: None,
            thread_store: None,
            context: None,
        }
    }

    pub fn with_registry(mut self, registry: Arc<RwLock<ServerRegistry>>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_tool_sessions(mut self, tool_sessions: Arc<Box<dyn ToolSessionStore>>) -> Self {
        self.tool_sessions = Some(tool_sessions);
        self
    }

    pub fn with_session_store(mut self, session_store: Arc<Box<dyn SessionStore>>) -> Self {
        self.session_store = Some(session_store);
        self
    }

    pub fn with_agent_store(mut self, agent_store: Arc<dyn AgentStore>) -> Self {
        self.agent_store = Some(agent_store);
        self
    }

    pub fn with_task_store(mut self, task_store: Arc<dyn TaskStore>) -> Self {
        self.task_store = Some(task_store);
        self
    }

    pub fn with_thread_store(mut self, thread_store: Arc<Box<dyn ThreadStore>>) -> Self {
        self.thread_store = Some(thread_store);
        self
    }

    pub fn with_context(mut self, context: Arc<ExecutorContext>) -> Self {
        self.context = Some(context);
        self
    }

    pub async fn initialize_stores_from_config(
        mut self,
        store_config: Option<&crate::types::StoreConfig>,
    ) -> anyhow::Result<Self> {
        let default_config = crate::types::StoreConfig::default();
        let config = store_config.unwrap_or(&default_config);

        // Initialize all stores using the StoreConfig::initialize method
        let stores = config.initialize().await?;

        // Set the stores if not already provided
        if self.session_store.is_none() {
            self.session_store = Some(stores.session_store);
        }
        if self.agent_store.is_none() {
            self.agent_store = Some(stores.agent_store);
        }
        if self.task_store.is_none() {
            self.task_store = Some(stores.task_store);
        }
        if self.thread_store.is_none() {
            self.thread_store = Some(stores.thread_store);
        }
        if self.tool_sessions.is_none() {
            self.tool_sessions = stores.tool_session_store;
        }

        // Initialize defaults for remaining fields
        if self.context.is_none() {
            self.context = Some(Arc::new(ExecutorContext::default()));
        }
        if self.registry.is_none() {
            self.registry = Some(Arc::new(RwLock::new(ServerRegistry::new())));
        }

        Ok(self)
    }

    pub fn build(self) -> anyhow::Result<AgentExecutor> {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);

        let registry = self
            .registry
            .ok_or_else(|| anyhow::anyhow!("Registry is required"))?;
        let session_store = self
            .session_store
            .unwrap_or_else(|| Arc::new(Box::new(LocalSessionStore::new())));
        let agent_store = self
            .agent_store
            .ok_or_else(|| anyhow::anyhow!("Agent store is required"))?;
        let task_store = self
            .task_store
            .unwrap_or_else(|| Arc::new(HashMapTaskStore::new()));
        let thread_store = self.thread_store.unwrap_or_else(|| {
            Arc::new(Box::new(HashMapThreadStore::default()) as Box<dyn ThreadStore>)
        });
        let context = self
            .context
            .unwrap_or_else(|| Arc::new(ExecutorContext::default()));

        Ok(AgentExecutor {
            agent_store,
            tool_sessions: self.tool_sessions,
            registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            session_store,
            thread_store,
            task_store,
            context,
        })
    }
}

impl AgentExecutor {
    pub fn new(
        registry: Arc<RwLock<ServerRegistry>>,
        tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
        session_store: Option<Arc<Box<dyn SessionStore>>>,
        agent_store: Arc<dyn AgentStore>,
        context: Arc<ExecutorContext>,
    ) -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        let thread_store =
            Arc::new(Box::new(HashMapThreadStore::default()) as Box<dyn ThreadStore>);

        Self {
            agent_store: agent_store,
            tool_sessions,
            registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            session_store: session_store
                .unwrap_or_else(|| Arc::new(Box::new(LocalSessionStore::new()))),
            thread_store,
            task_store: Arc::new(HashMapTaskStore::new()),
            context: context,
        }
    }

    /// Initialize AgentExecutor from configuration
    pub async fn initialize(
        config: &Configuration,
        servers: HashMap<String, ServerMetadata>,
    ) -> anyhow::Result<Arc<AgentExecutor>> {
        let mut builder = AgentExecutorBuilder::new();

        let registry = Arc::new(RwLock::new(ServerRegistry::new()));
        // Initialize stores from configuration
        builder = builder
            .initialize_stores_from_config(config.stores.as_ref())
            .await?;

        let executor = Arc::new(builder.build()?);
        register_servers(registry, executor.clone(), servers).await?;

        // Register agents from configuration
        for agent_config in &config.agents {
            executor
                .register_default_agent(agent_config.definition.clone())
                .await?;
        }

        Ok(executor)
    }

    /// Initialize AgentExecutor from configuration string or file path
    pub async fn initialize_from_config(
        config_source: &str,
        servers: HashMap<String, ServerMetadata>,
    ) -> anyhow::Result<Arc<AgentExecutor>> {
        let config = if std::path::Path::new(config_source).exists() {
            // It's a file path
            let config_str = std::fs::read_to_string(config_source)?;
            serde_yaml::from_str::<Configuration>(&config_str)?
        } else {
            // It's a config string
            serde_yaml::from_str::<Configuration>(config_source)?
        };

        Self::initialize(&config, servers).await
    }

    pub async fn execute_tool(
        &self,
        agent_id: &str,
        tool_call: ToolCall,
    ) -> Result<String, AgentError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.coordinator_tx
            .send(CoordinatorMessage::ExecuteTool {
                agent_id: agent_id.to_string(),
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

    pub async fn register_agent(&self, agent: Box<dyn BaseAgent>) -> anyhow::Result<()> {
        tracing::info!("Registering agent: {}", agent.get_name());

        self.agent_store
            .register(agent)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    /// Helper method to create a DefaultAgent from an AgentDefinition
    pub async fn register_default_agent(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let resolved_tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;
        let agent = Box::new(StandardAgent::new(
            definition,
            resolved_tools,
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
        ));
        self.register_agent(agent.clone_box()).await?;
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
                            let agent = agent_store.get(&agent_id).await;

                            match agent {
                                Some(agent) => {
                                    // Execute the tool
                                    let server_tools = agent.get_tools();
                                    let tool_def = server_tools
                                        .iter()
                                        .find(|t| {
                                            t.tools
                                                .iter()
                                                .any(|tool| tool.name == tool_call.tool_name)
                                        })
                                        .cloned();

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
                    event_tx,
                    response_tx,
                } => {
                    tracing::info!(
                        "Handling Execute for agent: {} with messages: {:?}",
                        agent_id,
                        task
                    );
                    let this = self.clone();
                    tokio::spawn(async move {
                        let result = async {
                            this.call_agent(&agent_id, task, params, context, event_tx)
                                .await
                        }
                        .await;

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
                            tracing::error!("Error in Coordinator:ExecuteStream: {}", e);
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
        context: Arc<ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        // Get agent from store and use invoke method
        if let Some(agent) = self.agent_store.get(agent_id).await {
            agent.invoke(task, params, context, event_tx).await
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
        context: Arc<ExecutorContext>,
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

    pub async fn execute(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.call_agent(agent_name, task, params, context, event_tx)
            .await
    }

    pub async fn execute_stream(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        event_tx: mpsc::Sender<AgentEvent>,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        self.call_agent_stream(agent_name, task, params, context, event_tx)
            .await
    }
}
