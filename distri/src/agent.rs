use crate::{
    coordinator::{AgentCoordinator, AgentEvent, CoordinatorContext},
    error::AgentError,
    memory::TaskStep,
    types::BaseAgent,
};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Abstract Agent that contains the main execution logic
pub struct AbstractAgent<C: AgentCoordinator> {
    coordinator: Arc<C>,
}

impl<C: AgentCoordinator> AbstractAgent<C> {
    pub fn new(coordinator: Arc<C>) -> Self {
        Self { coordinator }
    }

    /// Execute an agent with optional custom BaseAgent implementation
    pub async fn execute_agent(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        base_agent: Option<&dyn BaseAgent>,
    ) -> Result<String, AgentError> {
        // Call pre_execution hook if custom agent is provided
        if let Some(agent) = base_agent {
            agent
                .pre_execution(agent_id, &task, params.as_ref(), context.clone())
                .await?;
        }

        // Execute the main agent logic
        let result = self
            .execute_agent_core(agent_id, task.clone(), params.clone(), context.clone())
            .await;

        // Call post_execution hook if custom agent is provided
        if let Some(agent) = base_agent {
            agent
                .post_execution(agent_id, &task, params.as_ref(), context.clone(), &result)
                .await?;
        }

        result
    }

    /// Execute an agent with streaming and optional custom BaseAgent implementation
    pub async fn execute_agent_stream(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
        base_agent: Option<&dyn BaseAgent>,
    ) -> Result<(), AgentError> {
        // Call pre_execution hook if custom agent is provided
        if let Some(agent) = base_agent {
            agent
                .pre_execution(agent_id, &task, params.as_ref(), context.clone())
                .await?;
        }

        // Execute the main agent logic with streaming
        let result = self
            .execute_agent_stream_core(
                agent_id,
                task.clone(),
                params.clone(),
                context.clone(),
                event_tx,
            )
            .await;

        // Call post_execution hook if custom agent is provided
        if let Some(agent) = base_agent {
            // For streaming, we need to convert the Result<(), AgentError> to Result<String, AgentError>
            let string_result = match &result {
                Ok(_) => Ok(String::new()),
                Err(e) => Err(e.clone()),
            };
            agent
                .post_execution(agent_id, &task, params.as_ref(), context.clone(), &string_result)
                .await?;
        }

        result
    }

    /// Core agent execution logic (extracted from LocalCoordinator::call_agent)
    async fn execute_agent_core(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<String, AgentError> {
        // Get agent definition and tools
        let _definition = self.coordinator.get_agent(agent_id).await?;
        let _tools = self.coordinator.get_tools(agent_id).await?;

        // This would need access to session_store and logger from coordinator
        // For now, let's delegate back to the coordinator's execute method
        // In a full implementation, we'd move the session store and logger logic here
        self.coordinator
            .execute(agent_id, task, params, context)
            .await
    }

    /// Core agent streaming execution logic (extracted from LocalCoordinator::call_agent_stream)
    async fn execute_agent_stream_core(
        &self,
        agent_id: &str,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        // For now, delegate to the coordinator's execute_stream method
        // In a full implementation, we'd move the session store and logger logic here
        self.coordinator
            .execute_stream(agent_id, task, params, event_tx, context)
            .await
    }
}