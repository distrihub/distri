use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::mpsc;
use async_openai::types::CreateChatCompletionResponse;

use crate::{
    coordinator::{AgentCoordinator, AgentEvent, CoordinatorContext},
    error::AgentError,
    memory::TaskStep,
    types::{Agent, AgentDefinition, BaseAgent, Message, RunnableAgent},
};

/// Local agent implementation that runs the existing agent logic
pub struct LocalAgent {
    pub definition: AgentDefinition,
}

impl LocalAgent {
    pub fn new(definition: AgentDefinition) -> Self {
        Self { definition }
    }
}

#[async_trait]
impl BaseAgent for LocalAgent {
    async fn plan(
        &self,
        _task: &TaskStep,
        _coordinator: &dyn AgentCoordinator,
    ) -> Result<(), AgentError> {
        // Planning is handled in invoke/invoke_stream for Local agents
        Ok(())
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        coordinator: &dyn AgentCoordinator,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        // This is the existing call_agent logic from LocalCoordinator
        // We'll need to extract the memory store and other dependencies from coordinator
        
        // For now, delegate back to the coordinator's execute method
        // This maintains backward compatibility while we refactor
        coordinator.execute(&self.definition.name, task, params, context).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        coordinator: &dyn AgentCoordinator,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Delegate to coordinator's execute_stream method
        coordinator.execute_stream(&self.definition.name, task, params, event_tx, context).await
    }
}

/// Remote agent implementation (placeholder for future implementation)
pub struct RemoteAgent {
    pub url: String,
}

impl RemoteAgent {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait]
impl BaseAgent for RemoteAgent {
    async fn plan(
        &self,
        _task: &TaskStep,
        _coordinator: &dyn AgentCoordinator,
    ) -> Result<(), AgentError> {
        // TODO: Implement remote planning
        todo!("Remote agent planning not implemented")
    }

    async fn invoke(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _coordinator: &dyn AgentCoordinator,
        _context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        // TODO: Implement remote agent execution
        todo!("Remote agent execution not implemented")
    }

    async fn invoke_stream(
        &self,
        _task: TaskStep,
        _params: Option<serde_json::Value>,
        _coordinator: &dyn AgentCoordinator,
        _context: Arc<CoordinatorContext>,
        _event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // TODO: Implement remote agent streaming
        todo!("Remote agent streaming not implemented")
    }
}

/// Default runnable agent implementation that behaves like Local but allows custom step logic
pub struct DefaultRunnableAgent {
    pub definition: AgentDefinition,
}

impl DefaultRunnableAgent {
    pub fn new(definition: AgentDefinition) -> Self {
        Self { definition }
    }
}

#[async_trait]
impl RunnableAgent for DefaultRunnableAgent {
    async fn step<F, Fut>(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        _context: Arc<CoordinatorContext>,
        llm: F,
    ) -> Result<CreateChatCompletionResponse, AgentError>
    where
        F: Fn(&[Message], Option<serde_json::Value>) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<CreateChatCompletionResponse, AgentError>> + Send,
    {
        // Default implementation just calls the LLM directly
        llm(messages, params).await
    }
}

#[async_trait]
impl BaseAgent for DefaultRunnableAgent {
    async fn plan(
        &self,
        _task: &TaskStep,
        _coordinator: &dyn AgentCoordinator,
    ) -> Result<(), AgentError> {
        // Planning is handled in invoke/invoke_stream for Runnable agents
        Ok(())
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        coordinator: &dyn AgentCoordinator,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        // Similar to Local agent but with custom step logic
        // For now, delegate to coordinator but this will be refactored
        coordinator.execute(&self.definition.name, task, params, context).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        coordinator: &dyn AgentCoordinator,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // Delegate to coordinator's execute_stream method
        coordinator.execute_stream(&self.definition.name, task, params, event_tx, context).await
    }
}

/// Factory function to create the appropriate agent implementation
pub fn create_agent(agent: &Agent) -> Box<dyn BaseAgent> {
    match agent {
        Agent::Local(definition) => Box::new(LocalAgent::new(definition.clone())),
        Agent::Remote(url) => Box::new(RemoteAgent::new(url.clone())),
        Agent::Runnable(definition) => Box::new(DefaultRunnableAgent::new(definition.clone())),
    }
}