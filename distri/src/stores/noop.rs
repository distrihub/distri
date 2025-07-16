use async_trait::async_trait;

use crate::{
    agent::ExecutorContext,
    memory::MemoryStep,
    types::{
        CreateThreadRequest, McpSession, Message, Task, TaskStatus, Thread, ThreadSummary,
        UpdateThreadRequest,
    },
    AgentStore, MemoryStore, SessionMemory, SessionStore, TaskStore, ThreadStore, ToolSessionStore,
};
use distri_a2a::Artifact;

// Noop ToolSessionStore
#[derive(Default)]
pub struct NoopToolSessionStore;

#[async_trait]
impl ToolSessionStore for NoopToolSessionStore {
    async fn get_session(
        &self,
        _server_name: &str,
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(None)
    }
}

// Noop SessionStore
#[derive(Clone, Default)]
pub struct NoopSessionStore;

#[async_trait::async_trait]
impl SessionStore for NoopSessionStore {
    async fn get_steps(&self, _thread_id: &str) -> anyhow::Result<Vec<MemoryStep>> {
        Ok(vec![])
    }

    async fn store_step(&self, _thread_id: &str, _step: MemoryStep) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_session(&self, _thread_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn inc_iteration(&self, _thread_id: &str) -> anyhow::Result<i32> {
        Ok(0)
    }

    async fn get_iteration(&self, _thread_id: &str) -> anyhow::Result<i32> {
        Ok(0)
    }
}

// Noop MemoryStore
#[derive(Clone, Default)]
pub struct NoopMemoryStore;

#[async_trait::async_trait]
impl MemoryStore for NoopMemoryStore {
    async fn store_memory(
        &self,
        _user_id: &str,
        _session_memory: SessionMemory,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn search_memories(
        &self,
        _user_id: &str,
        _query: &str,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn get_user_memories(&self, _user_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn clear_user_memories(&self, _user_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

// Noop TaskStore
#[derive(Clone, Default)]
pub struct NoopTaskStore;

#[async_trait]
impl TaskStore for NoopTaskStore {
    async fn create_task(&self, context_id: &str, task_id: Option<&str>) -> anyhow::Result<Task> {
        let task = TaskStore::init_task(self, context_id, task_id);
        Ok(task)
    }

    async fn get_task(&self, _task_id: &str) -> anyhow::Result<Option<Task>> {
        Ok(None)
    }

    async fn update_task_status(&self, _task_id: &str, _status: TaskStatus) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_task(&self, _task_id: &str) -> anyhow::Result<Task> {
        Err(anyhow::anyhow!("Task not found"))
    }

    async fn add_message_to_task(&self, _task_id: &str, _message: &Message) -> anyhow::Result<()> {
        Ok(())
    }

    async fn add_artifact_to_task(
        &self,
        _task_id: &str,
        _artifact: Artifact,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_tasks(&self, _context_id: Option<&str>) -> anyhow::Result<Vec<Task>> {
        Ok(vec![])
    }

    async fn get_messages(&self, _thread_id: &str) -> anyhow::Result<Vec<Message>> {
        Ok(vec![])
    }
}

// Noop ThreadStore
#[derive(Default)]
pub struct NoopThreadStore;

#[async_trait]
impl ThreadStore for NoopThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread> {
        Ok(Thread::new(
            request.agent_id,
            request.title,
            request.thread_id,
        ))
    }

    async fn get_thread(&self, _thread_id: &str) -> anyhow::Result<Option<Thread>> {
        Ok(None)
    }

    async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> anyhow::Result<Thread> {
        Ok(Thread::new(
            String::from("noop-agent"),
            request.title.clone(),
            Some(thread_id.to_string()),
        ))
    }

    async fn delete_thread(&self, _thread_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_threads(
        &self,
        _agent_id: Option<&str>,
        _limit: Option<u32>,
        _offset: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadSummary>> {
        Ok(vec![])
    }

    async fn update_thread_with_message(
        &self,
        _thread_id: &str,
        _message: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// Noop AgentStore
#[derive(Default)]
pub struct NoopAgentStore;

#[async_trait]
impl AgentStore for NoopAgentStore {
    async fn list(
        &self,
        _cursor: Option<String>,
        _limit: Option<usize>,
    ) -> (Vec<crate::types::AgentDefinition>, Option<String>) {
        (vec![], None)
    }

    async fn get(&self, _name: &str) -> Option<crate::types::AgentDefinition> {
        None
    }

    async fn register(&self, _definition: crate::types::AgentDefinition) -> anyhow::Result<()> {
        Ok(())
    }

    async fn update(&self, _definition: crate::types::AgentDefinition) -> anyhow::Result<()> {
        Ok(())
    }
}
