use crate::{
    agent::{BaseAgent, StandardAgent, agent::AgentType},
    memory::TaskStep,
    types::AgentDefinition,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// A wrapper for custom agents that delegates to a StandardAgent
pub struct CustomAgentWrapper {
    inner: StandardAgent,
}

impl CustomAgentWrapper {
    pub fn new(inner: StandardAgent) -> Self {
        Self { inner }
    }
}

impl std::fmt::Debug for CustomAgentWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomAgentWrapper")
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait]
impl BaseAgent for CustomAgentWrapper {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("custom_wrapper".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.inner.get_definition()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.inner.get_tools()
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(Self {
            inner: self.inner.clone(),
        })
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        // Simply delegate to inner agent
        self.inner.invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), crate::error::AgentError> {
        // Simply delegate to inner agent
        self.inner.invoke_stream(task, params, context, event_tx).await
    }
}