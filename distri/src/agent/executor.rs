use crate::{
    agent::{AgentEventType, AgentFactoryRegistry, BaseAgent},
    error::AgentError,
    servers::registry::{McpServerRegistry, ServerMetadata},
    stores::{AgentStore, ThreadStore, ToolSessionStore},
    tools::{get_tools, BuiltInToolContext, LlmToolsRegistry},
    types::{
        Configuration, CreateThreadRequest, StoreConfig, Thread, ThreadSummary, ToolCall,
        UpdateThreadRequest,
    },
    InitializedStores, SessionStore, TaskStore,
};
use serde_json;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

use super::AgentEvent;
use super::CoordinatorMessage;
use super::ExecutorContext;
use crate::memory::TaskStep;

// Message types for coordinator communication

#[derive(Clone)]
pub struct AgentExecutor {
    pub agent_store: Arc<dyn AgentStore>,
    pub tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    pub registry: Arc<RwLock<McpServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub session_store: Arc<Box<dyn SessionStore>>,
    pub thread_store: Arc<dyn ThreadStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub context: Arc<ExecutorContext>,
    pub agent_factory: Arc<RwLock<AgentFactoryRegistry>>,
}

#[derive(Default)]
pub struct AgentExecutorBuilder {
    registry: Option<Arc<RwLock<McpServerRegistry>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
    agent_store: Option<Arc<dyn AgentStore>>,
    task_store: Option<Arc<dyn TaskStore>>,
    thread_store: Option<Arc<dyn ThreadStore>>,
    context: Option<Arc<ExecutorContext>>,
    agent_factory: Option<Arc<RwLock<AgentFactoryRegistry>>>,
}

impl AgentExecutorBuilder {
    pub fn with_registry(mut self, registry: Arc<RwLock<McpServerRegistry>>) -> Self {
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

    pub fn with_thread_store(mut self, thread_store: Arc<dyn ThreadStore>) -> Self {
        self.thread_store = Some(thread_store);
        self
    }

    pub fn with_context(mut self, context: Arc<ExecutorContext>) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_agent_factory(mut self, agent_factory: Arc<RwLock<AgentFactoryRegistry>>) -> Self {
        self.agent_factory = Some(agent_factory);
        self
    }

    pub fn with_stores(mut self, stores: InitializedStores) -> Self {
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
        self
    }

    pub fn build(self) -> anyhow::Result<AgentExecutor> {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);

        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(RwLock::new(McpServerRegistry::default())));

        let session_store = self
            .session_store
            .ok_or_else(|| anyhow::anyhow!("Session store is required. Use with_stores() to initialize. Default is LocalSessionStore."))?;

        let agent_store = self.agent_store.ok_or_else(|| {
            anyhow::anyhow!("Agent store is required. Use with_stores() to initialize. Default is InMemoryAgentStore.")
        })?;

        let task_store = self
            .task_store
            .ok_or_else(|| anyhow::anyhow!("Task store is required. Use with_stores() to initialize. Default is HashMapTaskStore."))?;
        let thread_store = self
            .thread_store
            .ok_or_else(|| anyhow::anyhow!("Thread store is required. Use with_stores() to initialize. Default is HashMapThreadStore."))?;

        let context = self
            .context
            .unwrap_or_else(|| Arc::new(ExecutorContext::default()));

        let agent_factory = self
            .agent_factory
            .unwrap_or_else(|| Arc::new(RwLock::new(AgentFactoryRegistry::default())));

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
            agent_factory,
        })
    }
}

impl AgentExecutor {
    /// Initialize AgentExecutor from configuration
    pub async fn initialize(config: &Configuration) -> anyhow::Result<Self> {
        let builder = AgentExecutorBuilder::default();

        let store_config = config.stores.clone().unwrap_or(StoreConfig::default());
        // Initialize stores from configuration

        let stores = store_config.initialize().await?;
        let builder = builder.with_stores(stores);

        let executor = builder.build()?;

        // Register agents from configuration
        for definition in &config.agents {
            executor
                .register_agent_definition(definition.clone())
                .await?;
        }

        Ok(executor)
    }

    pub async fn register_mcp_server(&self, name: String, server: ServerMetadata) {
        let registry = self.registry.clone();
        registry.write().await.register(name, server);
    }

    pub async fn execute_tool(
        &self,
        agent_id: String,
        tool_call: ToolCall,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
        context: Arc<ExecutorContext>,
    ) -> Result<String, AgentError> {
        // Get agent definition and create agent instance
        let definition = self
            .agent_store
            .get(&agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

        let agent = self.create_agent_from_definition(definition).await?;

        let tools = agent.get_tools();

        if let Some(tool) = tools.iter().find(|t| t.get_name() == tool_call.tool_name) {
            let registry = self.registry.clone();
            let tool_sessions = self.tool_sessions.clone();
            let context = context.clone();
            let event_tx = event_tx.clone();

            let tool_context = BuiltInToolContext {
                agent_id: agent_id.clone(),
                agent_store: self.agent_store.clone(),
                context: context.clone(),
                event_tx,
                coordinator_tx: self.coordinator_tx.clone(),
                tool_sessions,
                registry,
            };

            tracing::info!("Executing tool call: {:#?}", tool_call);
            let res = tool.execute(tool_call, tool_context).await;
            match res {
                Ok(content) => {
                    return Ok(content);
                }
                Err(e) => {
                    tracing::error!("Error executing tool: {}", e);
                    return Err(e);
                }
            }
        }
        Err(AgentError::ToolNotFound(tool_call.tool_name))
    }

    pub async fn register_agent_definition(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> anyhow::Result<()> {
        tracing::info!("Registering agent definition: {}", definition.name);

        self.agent_store
            .register(definition)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    /// Create an agent instance from a definition using the factory
    pub async fn create_agent_from_definition(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> Result<Box<dyn BaseAgent>, AgentError> {
        let tools = get_tools(&definition.mcp_servers, self.registry.clone())
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
        let tools_registry = LlmToolsRegistry::new(tools);

        let factory = self.agent_factory.read().await;
        factory.create_agent(
            definition,
            Arc::new(tools_registry),
            Arc::new(self.clone()),
            self.session_store.clone(),
        )
    }

    /// Register a custom agent factory
    pub async fn register_agent_factory(
        &self,
        agent_type: String,
        factory: Arc<crate::agent::factory::AgentFactoryFn>,
    ) {
        let mut factory_registry = self.agent_factory.write().await;
        factory_registry.register_factory(agent_type, factory);
    }

    /// Update an existing agent with new definition
    pub async fn update_agent_definition(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> anyhow::Result<()> {
        self.agent_store.update(definition).await
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        tracing::info!("AgentCoordinator run loop started");

        while let Some(msg) = self.coordinator_rx.lock().await.recv().await {
            tracing::info!("AgentCoordinator received a message: {:?}", msg);
            match msg {
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
                CoordinatorMessage::HandoverAgent {
                    from_agent,
                    to_agent,
                    reason,
                    context,
                    event_tx,
                } => {
                    tracing::info!(
                        "Handling agent handover from {} to {}",
                        from_agent,
                        to_agent
                    );

                    // Emit the AgentHandover event if event_tx is available
                    if let Some(event_tx) = event_tx {
                        let run_id = context.run_id.lock().await.clone();
                        let handover_event = AgentEvent {
                            thread_id: context.thread_id.clone(),
                            run_id,
                            event: AgentEventType::AgentHandover {
                                from_agent: from_agent.clone(),
                                to_agent: to_agent.clone(),
                                reason,
                            },
                        };

                        if let Err(e) = event_tx.send(handover_event).await {
                            tracing::error!("Failed to send AgentHandover event: {}", e);
                        } else {
                            tracing::info!(
                                "Successfully emitted AgentHandover event from {} to {}",
                                from_agent,
                                to_agent
                            );
                        }
                    }
                }
                CoordinatorMessage::ExecuteTools { .. } => {
                    return Err(anyhow::anyhow!("ExecuteTools not implemented"));
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
        // Get agent definition and create agent instance
        let definition = self
            .agent_store
            .get(agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

        let agent_type = definition
            .agent_type
            .clone()
            .unwrap_or("standard".to_string());
        let agent = self.create_agent_from_definition(definition).await?;
        tracing::info!(
            "Invoking agent: {}, {}, {:?}",
            agent_type,
            agent.get_name(),
            agent.agent_type()
        );
        agent.invoke(task, params, context, event_tx).await
    }

    async fn call_agent_stream(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Get agent definition and create agent instance
        let definition = self
            .agent_store
            .get(agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

        let agent_type = definition
            .agent_type
            .clone()
            .unwrap_or("standard".to_string());

        let agent: Box<dyn BaseAgent> = self.create_agent_from_definition(definition).await?;
        tracing::info!(
            "Invoking agent: {}, {}, {:?}",
            agent_type,
            agent.get_name(),
            agent.agent_type()
        );
        agent.invoke_stream(task, params, context, event_tx).await
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
        user_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ThreadSummary>, AgentError> {
        self.thread_store
            .list_threads(user_id, limit, offset)
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
