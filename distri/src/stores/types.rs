use async_trait::async_trait;

use crate::{
    agent::{BaseAgent, ExecutorContext},
    memory::MemoryStep,
    types::{CreateThreadRequest, McpSession, Message, Thread, ThreadSummary, UpdateThreadRequest},
};
use distri_a2a::{Artifact, Message as A2aMessage, Task, TaskStatus};

#[async_trait]
pub trait ToolSessionStore: Send + Sync {
    async fn get_session(
        &self,
        server_name: &str,
        context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>>;
}

// SessionStore trait - manages current conversation thread/run
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    async fn get_messages(&self, thread_id: &str) -> anyhow::Result<Vec<Message>> {
        let steps = self.get_steps(thread_id).await?;
        let messages = steps
            .iter()
            .flat_map(|step| step.to_messages(false, false))
            .collect();
        Ok(messages)
    }

    async fn get_steps(&self, thread_id: &str) -> anyhow::Result<Vec<MemoryStep>>;

    async fn store_step(&self, thread_id: &str, step: MemoryStep) -> anyhow::Result<()>;

    async fn clear_session(&self, thread_id: &str) -> anyhow::Result<()>;

    async fn inc_iteration(&self, run_id: &str) -> anyhow::Result<i32>;

    async fn get_iteration(&self, run_id: &str) -> anyhow::Result<i32>;
}

// Higher-level MemoryStore trait - manages cross-session permanent memory using user_id
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()>;

    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>>;

    async fn get_user_memories(&self, user_id: &str) -> anyhow::Result<Vec<String>>;

    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMemory {
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

// TaskStore trait - manages A2A tasks
#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn create_task(&self, context_id: &str, task_id: Option<&str>) -> anyhow::Result<Task>;

    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;

    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;

    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()>;

    async fn add_artifact_to_task(&self, task_id: &str, artifact: Artifact) -> anyhow::Result<()>;

    async fn list_tasks(&self, context_id: Option<&str>) -> anyhow::Result<Vec<Task>>;
}

#[async_trait]
pub trait ThreadStore: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread>;
    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>>;
    async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> anyhow::Result<Thread>;
    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()>;
    async fn list_threads(
        &self,
        agent_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadSummary>>;
    async fn update_thread_with_message(
        &self,
        thread_id: &str,
        message: &str,
    ) -> anyhow::Result<()>;
}

/// Agent factory trait for creating custom agents
#[async_trait]
pub trait AgentFactory: Send + Sync {
    /// Create a custom agent from an agent definition and context
    async fn create_agent(
        &self,
        definition: crate::types::AgentDefinition,
        executor: std::sync::Arc<crate::agent::AgentExecutor>,
        context: std::sync::Arc<ExecutorContext>,
        session_store: std::sync::Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>>;

    /// Get the agent type this factory can create
    fn agent_type(&self) -> &str;
}

/// Agent metadata stored in the agent store
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentMetadata {
    pub name: String,
    pub agent_type: String,
    pub definition: crate::types::AgentDefinition,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Returns a page of agents and an optional next cursor
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<Box<dyn BaseAgent>>, Option<String>);
    
    async fn get(&self, name: &str) -> Option<Box<dyn BaseAgent>>;
    
    async fn register(&self, agent: Box<dyn BaseAgent>) -> anyhow::Result<()>;
    
    /// Update an existing agent with new definition
    async fn update(&self, agent: Box<dyn BaseAgent>) -> anyhow::Result<()>;

    /// Register a custom agent factory
    async fn register_factory(&self, factory: Box<dyn AgentFactory>) -> anyhow::Result<()>;

    /// Get agent metadata without resolving the full agent
    async fn get_metadata(&self, name: &str) -> Option<AgentMetadata>;
}
