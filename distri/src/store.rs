use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    coordinator::CoordinatorContext,
    memory::{LocalAgentMemory, MemoryStep},
    types::{AgentDefinition, McpSession, Message, ServerTools},
    error::AgentError,
};
use distri_a2a::{Message as A2aMessage, Task, TaskState, TaskStatus};

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
    async fn create_task(
        &self,
        agent_id: &str,
        context_id: &str,
        kind: &str,
    ) -> anyhow::Result<Task>;
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
    async fn create_task(
        &self,
        agent_id: &str,
        context_id: &str,
        kind: &str,
    ) -> anyhow::Result<Task> {
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

// AgentStore trait for managing agent definitions and tools
#[async_trait]
pub trait AgentStore: Send + Sync {
    async fn register_agent(&self, definition: AgentDefinition, tools: Vec<ServerTools>) -> anyhow::Result<()>;
    async fn get_agent(&self, agent_name: &str) -> Result<AgentDefinition, AgentError>;
    async fn get_tools(&self, agent_name: &str) -> Result<Vec<ServerTools>, AgentError>;
    async fn list_agents(&self, cursor: Option<String>) -> Result<(Vec<AgentDefinition>, Option<String>), AgentError>;
    async fn has_agent(&self, agent_name: &str) -> bool;
}

// Local HashMap-based agent store implementation
#[derive(Clone)]
pub struct LocalAgentStore {
    pub agent_definitions: Arc<RwLock<HashMap<String, AgentDefinition>>>,
    pub agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
}

impl Default for LocalAgentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalAgentStore {
    pub fn new() -> Self {
        Self {
            agent_definitions: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl AgentStore for LocalAgentStore {
    async fn register_agent(&self, definition: AgentDefinition, tools: Vec<ServerTools>) -> anyhow::Result<()> {
        let name = definition.name.clone();
        
        // Store the definition
        {
            let mut definitions = self.agent_definitions.write().await;
            definitions.insert(name.clone(), definition);
        }

        // Store the tools
        {
            let mut agent_tools = self.agent_tools.write().await;
            agent_tools.insert(name, tools);
        }
        
        Ok(())
    }

    async fn get_agent(&self, agent_name: &str) -> Result<AgentDefinition, AgentError> {
        let definitions = self.agent_definitions.read().await;
        definitions
            .get(agent_name)
            .cloned()
            .ok_or_else(|| AgentError::NotFound(format!("Agent '{}' not found", agent_name)))
    }

    async fn get_tools(&self, agent_name: &str) -> Result<Vec<ServerTools>, AgentError> {
        let tools = self.agent_tools.read().await;
        tools
            .get(agent_name)
            .cloned()
            .ok_or_else(|| AgentError::NotFound(format!("Tools for agent '{}' not found", agent_name)))
    }

    async fn list_agents(&self, _cursor: Option<String>) -> Result<(Vec<AgentDefinition>, Option<String>), AgentError> {
        let definitions = self.agent_definitions.read().await;
        let agents: Vec<AgentDefinition> = definitions.values().cloned().collect();
        Ok((agents, None))
    }

    async fn has_agent(&self, agent_name: &str) -> bool {
        let definitions = self.agent_definitions.read().await;
        definitions.contains_key(agent_name)
    }
}
