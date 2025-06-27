use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    coordinator::CoordinatorContext,
    memory::{LocalAgentMemory, MemoryStep},
    types::{McpSession, Message, Thread, ThreadSummary, CreateThreadRequest, UpdateThreadRequest},
};
use distri_a2a::{Task, TaskState, TaskStatus, Message as A2aMessage};

#[async_trait]
pub trait ToolSessionStore: Send + Sync {
    async fn get_session(
        &self,
        server_name: &str,
        context: &CoordinatorContext,
    ) -> anyhow::Result<Option<McpSession>>;
}

// Example in-memory implementation
#[derive(Default)]
pub struct InMemorySessionStore {
    mcp_sessions: HashMap<String, McpSession>,
}

impl InMemorySessionStore {
    pub fn new(mcp_sessions: HashMap<String, McpSession>) -> Self {
        Self { mcp_sessions }
    }
}

#[async_trait]
impl ToolSessionStore for InMemorySessionStore {
    async fn get_session(
        &self,
        server_name: &str,
        _context: &CoordinatorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(self.mcp_sessions.get(server_name).cloned())
    }
}

// Define trait for memory storage
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    async fn get_messages(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<Message>> {
        let steps = self.get_steps(agent_id, thread_id).await?;
        let messages = steps
            .iter()
            .flat_map(|step| step.to_messages(false, false))
            .collect();
        Ok(messages)
    }
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>>;
    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()>;
}

// Local implementation using HashMap
#[derive(Clone)]
pub struct LocalMemoryStore {
    memories: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
}

impl Default for LocalMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalMemoryStore {
    pub fn new() -> Self {
        Self {
            memories: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MemoryStore for LocalMemoryStore {
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>> {
        let memories = self.memories.read().await;
        let memory = memories
            .get(agent_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::default);
        Ok(memory.get_steps(thread_id))
    }

    async fn store_step(
        &self,
        agent_id: &str,
        step: MemoryStep,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        let memory = memories
            .entry(agent_id.to_string())
            .or_insert_with(LocalAgentMemory::default);
        memory.add_step(step, thread_id);
        Ok(())
    }
}

// Task Store trait for A2A task management
#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn create_task(&self, agent_id: &str, context_id: &str, kind: &str) -> anyhow::Result<Task>;
    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;
    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;
    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()>;
    async fn list_tasks(&self, agent_id: Option<&str>) -> anyhow::Result<Vec<Task>>;
}

// HashMap-based task store implementation
#[derive(Clone)]
pub struct HashMapTaskStore {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
}

impl Default for HashMapTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl HashMapTaskStore {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl TaskStore for HashMapTaskStore {
    async fn create_task(&self, _agent_id: &str, context_id: &str, kind: &str) -> anyhow::Result<Task> {
        let task_id = Uuid::new_v4().to_string();
        let task = Task {
            id: task_id.clone(),
            kind: kind.to_string(),
            context_id: context_id.to_string(),
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
            artifacts: vec![],
            history: vec![],
        };

        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id, task.clone());
        Ok(task)
    }

    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.get(task_id).cloned())
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = status;
        }
        Ok(())
    }

    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus {
                state: TaskState::Canceled,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            };
            Ok(task.clone())
        } else {
            Err(anyhow::anyhow!("Task not found"))
        }
    }

    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.history.push(message);
        }
        Ok(())
    }

    async fn list_tasks(&self, _agent_id: Option<&str>) -> anyhow::Result<Vec<Task>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.values().cloned().collect())
    }
}

// Thread Store trait for thread management
#[async_trait]
pub trait ThreadStore: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread>;
    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>>;
    async fn update_thread(&self, thread_id: &str, request: UpdateThreadRequest) -> anyhow::Result<Thread>;
    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()>;
    async fn list_threads(&self, agent_id: Option<&str>, limit: Option<u32>, offset: Option<u32>) -> anyhow::Result<Vec<ThreadSummary>>;
    async fn update_thread_with_message(&self, thread_id: &str, message: &str) -> anyhow::Result<()>;
}

// HashMap-based thread store implementation
#[derive(Clone)]
pub struct HashMapThreadStore {
    threads: Arc<RwLock<HashMap<String, Thread>>>,
    agent_definitions: Arc<RwLock<HashMap<String, crate::types::AgentDefinition>>>,
}

impl Default for HashMapThreadStore {
    fn default() -> Self {
        Self::new()
    }
}

impl HashMapThreadStore {
    pub fn new() -> Self {
        Self {
            threads: Arc::new(RwLock::new(HashMap::new())),
            agent_definitions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn set_agent_definitions(&self, agents: HashMap<String, crate::types::AgentDefinition>) {
        let mut agent_defs = self.agent_definitions.write().await;
        *agent_defs = agents;
    }
}

#[async_trait]
impl ThreadStore for HashMapThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread> {
        let mut thread = Thread::new(request.agent_id, request.title);
        
        // If there's an initial message, update the thread with it
        if let Some(initial_message) = &request.initial_message {
            thread.update_with_message(initial_message);
        }
        
        let mut threads = self.threads.write().await;
        threads.insert(thread.id.clone(), thread.clone());
        Ok(thread)
    }

    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>> {
        let threads = self.threads.read().await;
        Ok(threads.get(thread_id).cloned())
    }

    async fn update_thread(&self, thread_id: &str, request: UpdateThreadRequest) -> anyhow::Result<Thread> {
        let mut threads = self.threads.write().await;
        let thread = threads.get_mut(thread_id)
            .ok_or_else(|| anyhow::anyhow!("Thread not found"))?;
        
        if let Some(title) = request.title {
            thread.title = title;
        }
        
        if let Some(metadata) = request.metadata {
            thread.metadata = metadata;
        }
        
        thread.updated_at = chrono::Utc::now();
        Ok(thread.clone())
    }

    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut threads = self.threads.write().await;
        threads.remove(thread_id);
        Ok(())
    }

    async fn list_threads(&self, agent_id: Option<&str>, limit: Option<u32>, offset: Option<u32>) -> anyhow::Result<Vec<ThreadSummary>> {
        let threads = self.threads.read().await;
        let agent_defs = self.agent_definitions.read().await;
        
        let mut thread_list: Vec<ThreadSummary> = threads
            .values()
            .filter(|thread| {
                agent_id.map_or(true, |aid| thread.agent_id == aid)
            })
            .map(|thread| {
                let agent_name = agent_defs.get(&thread.agent_id)
                    .map(|def| def.name.clone())
                    .unwrap_or_else(|| thread.agent_id.clone());
                
                ThreadSummary {
                    id: thread.id.clone(),
                    title: thread.title.clone(),
                    agent_id: thread.agent_id.clone(),
                    agent_name,
                    updated_at: thread.updated_at,
                    message_count: thread.message_count,
                    last_message: thread.last_message.clone(),
                }
            })
            .collect();
        
        // Sort by updated_at descending (most recent first)
        thread_list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        
        // Apply offset and limit
        let offset = offset.unwrap_or(0) as usize;
        let limit = limit.unwrap_or(50) as usize;
        
        let end = std::cmp::min(offset + limit, thread_list.len());
        if offset >= thread_list.len() {
            return Ok(vec![]);
        }
        
        Ok(thread_list[offset..end].to_vec())
    }

    async fn update_thread_with_message(&self, thread_id: &str, message: &str) -> anyhow::Result<()> {
        let mut threads = self.threads.write().await;
        if let Some(thread) = threads.get_mut(thread_id) {
            thread.update_with_message(message);
        }
        Ok(())
    }
}
