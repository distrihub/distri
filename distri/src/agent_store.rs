use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    coordinator::{AgentCoordinator, AgentEvent, CoordinatorContext},
    error::AgentError,
    memory::TaskStep,
    types::{AgentDefinition, ServerTools},
};

/// Agent store that manages all types of agents
#[derive(Clone)]
pub struct AgentStore {
    /// Local agents defined by YAML configuration
    pub local_agents: Arc<RwLock<HashMap<String, AgentDefinition>>>,
    /// Remote agents (A2A URLs)
    pub remote_agents: Arc<RwLock<HashMap<String, String>>>,
    /// Runnable agents with custom implementations
    pub runnable_agents: Arc<RwLock<HashMap<String, RunnableAgent>>>,
    /// Tools associated with each agent
    pub agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
}

impl Default for AgentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentStore {
    pub fn new() -> Self {
        Self {
            local_agents: Arc::new(RwLock::new(HashMap::new())),
            remote_agents: Arc::new(RwLock::new(HashMap::new())),
            runnable_agents: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a local agent (YAML-configured)
    pub async fn register_local_agent(
        &self,
        definition: AgentDefinition,
        tools: Vec<ServerTools>,
    ) -> Result<(), AgentError> {
        let name = definition.name.clone();
        
        {
            let mut local_agents = self.local_agents.write().await;
            local_agents.insert(name.clone(), definition);
        }
        
        {
            let mut agent_tools = self.agent_tools.write().await;
            agent_tools.insert(name, tools);
        }
        
        Ok(())
    }

    /// Register a remote agent (A2A)
    pub async fn register_remote_agent(
        &self,
        name: String,
        url: String,
    ) -> Result<(), AgentError> {
        let mut remote_agents = self.remote_agents.write().await;
        remote_agents.insert(name, url);
        Ok(())
    }

    /// Register a runnable agent with custom implementation
    pub async fn register_runnable_agent(
        &self,
        definition: AgentDefinition,
        custom_agent: Box<dyn CustomAgent>,
        tools: Vec<ServerTools>,
    ) -> Result<(), AgentError> {
        let name = definition.name.clone();
        
        let runnable_agent = RunnableAgent::new(definition.clone(), custom_agent);
        
        {
            let mut runnable_agents = self.runnable_agents.write().await;
            runnable_agents.insert(name.clone(), runnable_agent);
        }
        
        {
            let mut agent_tools = self.agent_tools.write().await;
            agent_tools.insert(name, tools);
        }
        
        Ok(())
    }

    /// Get agent definition by name
    pub async fn get_agent_definition(&self, name: &str) -> Result<AgentDefinition, AgentError> {
        // Check local agents first
        {
            let local_agents = self.local_agents.read().await;
            if let Some(def) = local_agents.get(name) {
                return Ok(def.clone());
            }
        }
        
        // Check runnable agents
        {
            let runnable_agents = self.runnable_agents.read().await;
            if let Some(runnable) = runnable_agents.get(name) {
                return Ok(runnable.definition.clone());
            }
        }
        
        Err(AgentError::NotFound(format!("Agent {} not found", name)))
    }

    /// Get tools for an agent
    pub async fn get_agent_tools(&self, name: &str) -> Result<Vec<ServerTools>, AgentError> {
        let agent_tools = self.agent_tools.read().await;
        Ok(agent_tools.get(name).cloned().unwrap_or_default())
    }

    /// Check if agent is runnable (has custom implementation)
    pub async fn is_runnable(&self, name: &str) -> bool {
        let runnable_agents = self.runnable_agents.read().await;
        runnable_agents.contains_key(name)
    }

    /// Execute an agent
    pub async fn execute_agent<C: AgentCoordinator>(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        coordinator: Arc<C>,
    ) -> Result<String, AgentError> {
        if self.is_runnable(agent_name).await {
            // Execute runnable agent
            let runnable_agents = self.runnable_agents.read().await;
            if let Some(runnable_agent) = runnable_agents.get(agent_name) {
                return runnable_agent.execute(task, params, context, coordinator).await;
            }
        }
        
        // Fall back to coordinator execution for local/remote agents
        coordinator.execute(agent_name, task, params, context).await
    }

    /// Execute an agent with streaming
    pub async fn execute_agent_stream<C: AgentCoordinator>(
        &self,
        agent_name: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
        coordinator: Arc<C>,
    ) -> Result<(), AgentError> {
        if self.is_runnable(agent_name).await {
            // Execute runnable agent with streaming
            let runnable_agents = self.runnable_agents.read().await;
            if let Some(runnable_agent) = runnable_agents.get(agent_name) {
                return runnable_agent.execute_stream(task, params, context, event_tx, coordinator).await;
            }
        }
        
        // Fall back to coordinator execution for local/remote agents
        coordinator.execute_stream(agent_name, task, params, event_tx, context).await
    }

    /// List all agents
    pub async fn list_all_agents(&self) -> Vec<AgentDefinition> {
        let mut agents = Vec::new();
        
        // Add local agents
        {
            let local_agents = self.local_agents.read().await;
            agents.extend(local_agents.values().cloned());
        }
        
        // Add runnable agents
        {
            let runnable_agents = self.runnable_agents.read().await;
            for runnable in runnable_agents.values() {
                agents.push(runnable.definition.clone());
            }
        }
        
        agents
    }
}

/// Custom agent trait for implementing step-based execution
#[async_trait::async_trait]
pub trait CustomAgent: Send + Sync + std::fmt::Debug {
    /// Main execution step - custom agents implement their logic here
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError>;

    /// Support for downcasting (useful for testing)
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Execution context provided to custom agents
pub struct AgentExecutionContext {
    pub agent_id: String,
    pub task: TaskStep,
    pub params: Option<serde_json::Value>,
    pub coordinator_context: Arc<CoordinatorContext>,
    pub llm_executor: LLMExecutorWrapper,
    pub session_writer: SessionWriter,
}

/// Wrapper around LLMExecutor for custom agents
pub struct LLMExecutorWrapper {
    definition: AgentDefinition,
    tools: Vec<ServerTools>,
    context: Arc<CoordinatorContext>,
}

impl LLMExecutorWrapper {
    pub fn new(
        definition: AgentDefinition,
        tools: Vec<ServerTools>,
        context: Arc<CoordinatorContext>,
    ) -> Self {
        Self {
            definition,
            tools,
            context,
        }
    }

    /// Execute LLM with current messages from session
    pub async fn llm(&self, messages: Vec<crate::types::Message>) -> Result<String, AgentError> {
        let executor = crate::executor::LLMExecutor::new(
            self.definition.clone(),
            self.tools.clone(),
            self.context.clone(),
            None,
            Some(format!("{}:custom", self.definition.name)),
        );
        
        let response = executor.execute(&messages, None).await?;
        let content = crate::executor::LLMExecutor::extract_first_choice(&response);
        Ok(content)
    }

    /// Execute LLM with streaming
    pub async fn llm_stream(
        &self,
        messages: Vec<crate::types::Message>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        let executor = crate::executor::LLMExecutor::new(
            self.definition.clone(),
            self.tools.clone(),
            self.context.clone(),
            None,
            Some(format!("{}:custom", self.definition.name)),
        );
        
        executor.execute_stream(&messages, None, event_tx).await
    }
}

/// Session writer for custom agents to write data into session
pub struct SessionWriter {
    context: Arc<CoordinatorContext>,
    session_store: Arc<Box<dyn crate::store::SessionStore>>,
}

impl SessionWriter {
    pub fn new(
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn crate::store::SessionStore>>,
    ) -> Self {
        Self {
            context,
            session_store,
        }
    }

    /// Write a memory step to the session
    pub async fn write_step(&self, step: crate::memory::MemoryStep) -> Result<(), AgentError> {
        self.session_store
            .store_step(&self.context.thread_id, step)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    /// Get current messages from session
    pub async fn get_messages(&self) -> Result<Vec<crate::types::Message>, AgentError> {
        self.session_store
            .get_messages(&self.context.thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }
}

/// Runnable agent that contains the execution loop
pub struct RunnableAgent {
    pub definition: AgentDefinition,
    pub custom_agent: Box<dyn CustomAgent>,
}

impl RunnableAgent {
    pub fn new(definition: AgentDefinition, custom_agent: Box<dyn CustomAgent>) -> Self {
        Self {
            definition,
            custom_agent,
        }
    }

    /// Execute the runnable agent
    pub async fn execute<C: AgentCoordinator>(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        coordinator: Arc<C>,
    ) -> Result<String, AgentError> {
        // Get tools for this agent
        let tools = coordinator.get_tools(&self.definition.name).await?;

        // Create execution context
        let execution_context = self.create_execution_context(
            task,
            params,
            context,
            tools,
            coordinator,
        ).await?;

        // Execute the custom agent's step function
        self.custom_agent.step(&execution_context).await
    }

    /// Execute the runnable agent with streaming
    pub async fn execute_stream<C: AgentCoordinator>(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        _event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
        coordinator: Arc<C>,
    ) -> Result<(), AgentError> {
        // Get tools for this agent
        let tools = coordinator.get_tools(&self.definition.name).await?;

        // Create execution context
        let execution_context = self.create_execution_context(
            task,
            params,
            context.clone(),
            tools,
            coordinator,
        ).await?;

        // Execute the custom agent's step function
        let _result = self.custom_agent.step(&execution_context).await?;
        
        // For now, we don't stream the custom agent execution
        // In the future, we could provide streaming capabilities to custom agents
        Ok(())
    }

    async fn create_execution_context<C: AgentCoordinator>(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        tools: Vec<ServerTools>,
        _coordinator: Arc<C>,
    ) -> Result<AgentExecutionContext, AgentError> {
        // Create LLM executor wrapper
        let llm_executor = LLMExecutorWrapper::new(
            self.definition.clone(),
            tools,
            context.clone(),
        );

        // Get session store from coordinator
        // Note: This requires the coordinator to expose its session store
        // For now, create a new local session store
        let session_store = Arc::new(Box::new(crate::store::LocalSessionStore::new()) as Box<dyn crate::store::SessionStore>);
        let session_writer = SessionWriter::new(context.clone(), session_store);

        Ok(AgentExecutionContext {
            agent_id: self.definition.name.clone(),
            task,
            params,
            coordinator_context: context,
            llm_executor,
            session_writer,
        })
    }
}