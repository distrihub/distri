use async_trait::async_trait;
use distri_a2a::Artifact;
use uuid::Uuid;

use crate::{
    agent::ExecutorContext,
    types::{
        CreateThreadRequest, McpSession, Message, Task, TaskStatus, Thread, ThreadSummary,
        UpdateThreadRequest,
    },
};

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
    async fn clear_session(&self, thread_id: &str) -> anyhow::Result<()>;

    async fn set_value(&self, thread_id: &str, key: &str, value: &str) -> anyhow::Result<()>;

    async fn get_value(&self, thread_id: &str, key: &str) -> anyhow::Result<Option<String>>;

    async fn delete_value(&self, thread_id: &str, key: &str) -> anyhow::Result<()>;
}

// Higher-level MemoryStore trait - manages cross-session permanent memory using user_id
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store permanent memory from a session for cross-session access
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()>;

    /// Search for relevant memories across sessions for a user
    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>>;

    /// Get all permanent memories for a user
    async fn get_user_memories(&self, user_id: &str) -> anyhow::Result<Vec<String>>;

    /// Clear all memories for a user
    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct SessionMemory {
    pub agent_id: String,
    pub thread_id: String,
    pub session_summary: String,
    pub key_insights: Vec<String>,
    pub important_facts: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// Task Store trait for A2A task management
#[async_trait]
pub trait TaskStore: Send + Sync {
    fn init_task(&self, context_id: &str, task_id: Option<&str>) -> Task {
        let task_id = task_id.unwrap_or(&Uuid::new_v4().to_string()).to_string();
        Task {
            id: task_id,
            status: TaskStatus::Pending,
            messages: vec![],
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            thread_id: context_id.to_string(),
        }
    }
    async fn create_task(&self, context_id: &str, task_id: Option<&str>) -> anyhow::Result<Task>;
    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;
    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;
    async fn add_message_to_task(&self, task_id: &str, message: &Message) -> anyhow::Result<()>;
    async fn add_artifact_to_task(&self, task_id: &str, artifact: Artifact) -> anyhow::Result<()>;
    async fn list_tasks(&self, thread_id: Option<&str>) -> anyhow::Result<Vec<Task>>;
    async fn get_messages(&self, thread_id: &str) -> anyhow::Result<Vec<Message>>;
}

// Thread Store trait for thread management
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

#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Returns a page of agent definitions and an optional next cursor
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<crate::types::AgentDefinition>, Option<String>);
    async fn get(&self, name: &str) -> Option<crate::types::AgentDefinition>;
    async fn register(&self, definition: crate::types::AgentDefinition) -> anyhow::Result<()>;
    /// Update an existing agent with new definition
    async fn update(&self, definition: crate::types::AgentDefinition) -> anyhow::Result<()>;
}
