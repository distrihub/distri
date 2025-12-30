use std::sync::Arc;

use serde_json::Value;

use crate::{LlmDefinition, Message, ToolCall};
use anyhow::Result;

/// Trait for workflow runtime functions (session storage, agent calls, etc.)
#[async_trait::async_trait]
pub trait OrchestratorTrait: Send + Sync {
    /// Get a session value for a specific session
    async fn get_session_value(&self, session_id: &str, key: &str) -> Option<serde_json::Value>;

    /// Set a session value for a specific session
    async fn set_session_value(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()>;

    /// Call an agent via the orchestrator
    async fn call_agent(
        &self,
        session_id: &str,
        agent_name: &str,
        task: &str,
    ) -> anyhow::Result<String>;

    async fn call_tool(
        &self,
        session_id: &str,
        user_id: &str,
        tool_call: &ToolCall,
    ) -> anyhow::Result<serde_json::Value>;

    async fn llm_execute(
        &self,
        llm_def: LlmDefinition,
        llm_context: LLmContext,
    ) -> Result<serde_json::Value, anyhow::Error>;
}

#[derive(Debug, Default)]
pub struct LLmContext {
    pub thread_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub label: Option<String>,
    pub messages: Vec<Message>,
}
/// Reference wrapper for orchestrator that allows late binding
pub struct OrchestratorRef {
    inner: std::sync::RwLock<Option<Arc<dyn OrchestratorTrait>>>,
}

impl OrchestratorRef {
    /// Create a new orchestrator reference without an actual orchestrator
    pub fn new() -> Self {
        Self {
            inner: std::sync::RwLock::new(None),
        }
    }

    /// Set the actual orchestrator (called after orchestrator is created)
    pub fn set_orchestrator(&self, orchestrator: Arc<dyn OrchestratorTrait>) {
        let mut inner = self.inner.write().unwrap();
        *inner = Some(orchestrator);
    }

    /// Get a clone of the orchestrator if available
    fn get_orchestrator(&self) -> Option<Arc<dyn OrchestratorTrait>> {
        let inner = self.inner.read().unwrap();
        inner.clone()
    }
}

#[async_trait::async_trait]
impl OrchestratorTrait for OrchestratorRef {
    /// Get a session value for a specific session
    async fn get_session_value(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        if let Some(orchestrator) = self.get_orchestrator() {
            orchestrator.get_session_value(session_id, key).await
        } else {
            // If no orchestrator is set, return None
            None
        }
    }

    /// Set a session value for a specific session
    async fn set_session_value(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        if let Some(orchestrator) = self.get_orchestrator() {
            orchestrator.set_session_value(session_id, key, value).await
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    /// Call an agent via the orchestrator
    async fn call_agent(&self, session_id: &str, agent_name: &str, task: &str) -> Result<String> {
        if let Some(orchestrator) = self.get_orchestrator() {
            orchestrator.call_agent(session_id, agent_name, task).await
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    async fn call_tool(
        &self,
        session_id: &str,
        user_id: &str,
        tool_call: &ToolCall,
    ) -> Result<serde_json::Value> {
        if let Some(orchestrator) = self.get_orchestrator() {
            orchestrator.call_tool(session_id, user_id, tool_call).await
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    async fn llm_execute(
        &self,
        llm_def: crate::LlmDefinition,
        llm_context: LLmContext,
    ) -> Result<serde_json::Value, anyhow::Error> {
        if let Some(orchestrator) = self.get_orchestrator() {
            orchestrator.llm_execute(llm_def, llm_context).await
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }
}

pub struct MockOrchestrator;

#[async_trait::async_trait]
impl OrchestratorTrait for MockOrchestrator {
    async fn get_session_value(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        Some(Value::String(format!("{}:{}", session_id, key)))
    }

    async fn set_session_value(
        &self,
        _session_id: &str,
        _key: &str,
        _value: serde_json::Value,
    ) -> Result<()> {
        Ok(())
    }

    async fn call_agent(&self, session_id: &str, agent_name: &str, task: &str) -> Result<String> {
        Ok(format!(
            "mock response for agent: {}, task {}, session_id {}",
            agent_name, task, session_id
        ))
    }

    async fn call_tool(
        &self,
        session_id: &str,
        user_id: &str,
        tool_call: &ToolCall,
    ) -> Result<serde_json::Value> {
        Ok(Value::String(format!(
            "mock response for tool: {},  session_id {}, user_id: {}, tool_call {}",
            tool_call.tool_name, session_id, user_id, tool_call.tool_call_id
        )))
    }

    async fn llm_execute(
        &self,
        llm_def: crate::LlmDefinition,
        llm_context: LLmContext,
    ) -> Result<serde_json::Value, anyhow::Error> {
        Ok(Value::String(format!(
            "mock response for llm_execute, llm_def {:?}, llm_context: {:?}",
            llm_def, llm_context
        )))
    }
}
