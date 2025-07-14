use crate::{
    agent::{BaseAgent, ExecutorContext},
    stores::SessionStore,
    types::AgentDefinition,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Agent factory trait for creating custom agents
#[async_trait]
pub trait AgentFactory: Send + Sync {
    /// Create a custom agent from an agent definition and context
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<crate::agent::AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>>;

    /// Get the agent type this factory can create
    fn agent_type(&self) -> &str;
}