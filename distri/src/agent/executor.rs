use crate::{
    agent::{AgentEventType, BaseAgent, StandardAgent, LoggingAgent, FilteringAgent, factory::AgentFactory},
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
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

use super::AgentEvent;
use super::CoordinatorMessage;
use super::ExecutorContext;
use crate::memory::TaskStep;

// Message types for coordinator communication

#[derive(Clone)]
pub struct AgentExecutor {
    pub agent_store: Arc<dyn AgentStore>,
    pub tool_sessions: Option<Arc<dyn ToolSessionStore>>,
    pub registry: Arc<RwLock<McpServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<AgentEventType>>>,
    pub coordinator_tx: mpsc::Sender<AgentEventType>,
    pub session_store: Arc<Box<dyn SessionStore>>,
    pub thread_store: Arc<dyn ThreadStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub context: Arc<ExecutorContext>,
    pub agent_factories: Arc<RwLock<std::collections::HashMap<String, Box<dyn AgentFactory>>>>,
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
            agent_factories: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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
        for agent_config in &config.agents {
            executor
                .register_default_agent(agent_config.definition.clone())
                .await?;
        }

        Ok(executor)
    }

    /// Create a minimal AgentExecutor for agent resolution purposes
    /// This is used when resolving custom agents from the store
    pub fn new_minimal_for_resolution() -> Self {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(100);
        
        Self {
            agent_store: Arc::new(crate::stores::memory::InMemoryAgentStore::new()),
            tool_sessions: None,
            registry: Arc::new(RwLock::new(McpServerRegistry::default())),
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            session_store: Arc::new(Box::new(crate::stores::memory::LocalSessionStore::new()) as Box<dyn SessionStore>),
            thread_store: Arc::new(crate::stores::memory::HashMapThreadStore::new()),
            task_store: Arc::new(crate::stores::memory::HashMapTaskStore::new()),
            context: Arc::new(ExecutorContext::default()),
            agent_factories: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
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
        // Check if this is a built-in tool first

        let agent = self
            .agent_store
            .get(&agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

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
        let tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;

        let tools_registry = LlmToolsRegistry::new(tools);

        let agent = Box::new(StandardAgent::new(
            definition,
            Arc::new(tools_registry),
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
        ));
        self.register_agent(agent.clone_box()).await?;
        Ok(agent)
    }

    /// Helper method to create a LoggingAgent from an AgentDefinition
    pub async fn register_logging_agent(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;
        let tools_registry = LlmToolsRegistry::new(tools);

        let agent = Box::new(LoggingAgent::new(
            definition,
            Arc::new(tools_registry),
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
        ));
        self.register_agent(agent.clone_box()).await?;
        Ok(agent)
    }

    /// Helper method to create a FilteringAgent from an AgentDefinition
    pub async fn register_filtering_agent(
        &self,
        definition: crate::types::AgentDefinition,
        banned_words: Vec<String>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;
        let tools_registry = LlmToolsRegistry::new(tools);

        let agent = Box::new(FilteringAgent::new(
            definition,
            Arc::new(tools_registry),
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
            banned_words,
        ));
        self.register_agent(agent.clone_box()).await?;
        Ok(agent)
    }

    /// Register a custom agent factory
    pub async fn register_factory(&self, factory: Box<dyn AgentFactory>) -> anyhow::Result<()> {
        let mut factories = self.agent_factories.write().await;
        factories.insert(factory.agent_type().to_string(), factory);
        Ok(())
    }

    /// Initialize an agent from the store with context
    pub async fn initialize_agent(
        &self,
        name: &str,
        context: Arc<ExecutorContext>,
    ) -> anyhow::Result<Option<Box<dyn BaseAgent>>> {
        // First try to get from the store directly
        if let Some(agent) = self.agent_store.get(name).await {
            return Ok(Some(agent));
        }

        // If not found, try to initialize using factories
        let metadata = self.agent_store.get_metadata(name).await;
        if let Some(metadata) = metadata {
            let factories = self.agent_factories.read().await;
            if let Some(factory) = factories.get(&metadata.agent_type) {
                let agent = factory.create_agent(
                    metadata.definition,
                    Arc::new(self.clone()),
                    context,
                    self.session_store.clone(),
                ).await?;
                return Ok(Some(agent));
            }
        }

        Ok(None)
    }

    /// Helper method to register a custom agent with a specific agent type
    pub async fn register_custom_agent(
        &self,
        definition: crate::types::AgentDefinition,
        agent_type: &str,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a custom agent that will be resolved by the factory
        // We'll create a placeholder agent that gets replaced when retrieved
        let agent = Box::new(StandardAgent::new(
            definition.clone(),
            Arc::new(LlmToolsRegistry::new(std::collections::HashMap::new())),
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
        ));
        
        // Store the agent
        self.register_agent(agent.clone_box()).await?;
        
        // For now, we'll just return the agent as-is
        // The custom agent resolution will happen when the agent is retrieved from the store
        // and the factory is available to create the proper custom agent
        
        Ok(agent)
    }

    /// Update an existing agent with new definition
    pub async fn update_agent(
        &self,
        definition: crate::types::AgentDefinition,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let tools = get_tools(&definition.mcp_servers, self.registry.clone()).await?;

        let tools_registry = LlmToolsRegistry::new(tools);

        let agent = Box::new(StandardAgent::new(
            definition,
            Arc::new(tools_registry),
            Arc::new(self.clone()),
            self.context.clone(),
            self.session_store.clone(),
        ));
        self.agent_store.update(agent.clone_box()).await?;
        Ok(agent)
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
