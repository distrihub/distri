use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    agent::ExecutorContext,
    memory::{LocalAgentMemory, MemoryStep},
    types::{CreateThreadRequest, McpSession, Thread, ThreadSummary, UpdateThreadRequest},
    AgentStore, MemoryStore, SessionMemory, SessionStore, TaskStore, ThreadStore, ToolSessionStore,
};
use distri_a2a::{Artifact, EventKind, Message as A2aMessage, Task, TaskState, TaskStatus};

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
        _context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        Ok(self.mcp_sessions.get(server_name).cloned())
    }
}

// Local SessionStore implementation using HashMap with just thread_id
#[derive(Clone)]
pub struct LocalSessionStore {
    sessions: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
    iterations: Arc<RwLock<HashMap<String, i32>>>,
}

impl Default for LocalSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            iterations: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl SessionStore for LocalSessionStore {
    async fn get_steps(&self, thread_id: &str) -> anyhow::Result<Vec<MemoryStep>> {
        let sessions = self.sessions.read().await;
        let memory = sessions
            .get(thread_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::default);
        Ok(memory.get_steps())
    }

    async fn store_step(&self, thread_id: &str, step: MemoryStep) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        let memory = sessions
            .entry(thread_id.to_string())
            .or_insert_with(LocalAgentMemory::default);
        memory.add_step(step);
        Ok(())
    }

    async fn clear_session(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(thread_id);
        Ok(())
    }

    async fn inc_iteration(&self, thread_id: &str) -> anyhow::Result<i32> {
        let mut iterations = self.iterations.write().await;
        tracing::debug!(
            "Incrementing iteration for thread: {}, iterations: {:#?}",
            thread_id,
            iterations
        );
        let count = iterations.entry(thread_id.to_string()).or_insert(0);
        *count += 1;
        Ok(*count)
    }

    async fn get_iteration(&self, thread_id: &str) -> anyhow::Result<i32> {
        let iterations = self.iterations.read().await;
        Ok(*iterations.get(thread_id).unwrap_or(&0))
    }
}

// Local MemoryStore implementation for cross-session permanent memory using user_id
#[derive(Clone, Default)]
pub struct LocalMemoryStore {
    memories: Arc<RwLock<HashMap<String, Vec<String>>>>,
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
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        let user_memories = memories.entry(user_id.to_string()).or_insert_with(Vec::new);

        // Create a consolidated memory entry from the session
        let memory_entry = format!(
            "Agent: {} | Session: {} ({})\nSummary: {}\nInsights: {}\nFacts: {}",
            session_memory.agent_id,
            session_memory.thread_id,
            session_memory.timestamp.format("%Y-%m-%d %H:%M:%S"),
            session_memory.session_summary,
            session_memory.key_insights.join("; "),
            session_memory.important_facts.join("; ")
        );

        user_memories.push(memory_entry);
        Ok(())
    }

    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>> {
        let memories = self.memories.read().await;
        if let Some(user_memories) = memories.get(user_id) {
            let query_lower = query.to_lowercase();
            let mut relevant_memories: Vec<String> = user_memories
                .iter()
                .filter(|memory| memory.to_lowercase().contains(&query_lower))
                .cloned()
                .collect();

            if let Some(limit) = limit {
                relevant_memories.truncate(limit);
            }

            Ok(relevant_memories)
        } else {
            Ok(Vec::new())
        }
    }

    async fn get_user_memories(&self, user_id: &str) -> anyhow::Result<Vec<String>> {
        let memories = self.memories.read().await;
        Ok(memories.get(user_id).cloned().unwrap_or_default())
    }

    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        memories.remove(user_id);
        Ok(())
    }
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
    async fn create_task(&self, context_id: &str, task_id: Option<&str>) -> anyhow::Result<Task> {
        let task_id = task_id.unwrap_or(&Uuid::new_v4().to_string()).to_string();
        let task = Task {
            kind: EventKind::Task,
            id: task_id.clone(),
            context_id: context_id.to_string(),
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
            artifacts: vec![],
            history: vec![],
            metadata: None,
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
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        task.status = TaskStatus {
            state: TaskState::Canceled,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        };

        Ok(task.clone())
    }

    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.history.push(message);
        }
        Ok(())
    }

    async fn add_artifact_to_task(&self, task_id: &str, artifact: Artifact) -> anyhow::Result<()> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.artifacts.push(artifact);
        }
        Ok(())
    }
    async fn list_tasks(&self, context_id: Option<&str>) -> anyhow::Result<Vec<Task>> {
        let tasks = self.tasks.read().await;
        let result = if let Some(context_id) = context_id {
            tasks
                .values()
                .filter(|task| task.context_id == context_id)
                .cloned()
                .collect()
        } else {
            tasks.values().cloned().collect()
        };
        Ok(result)
    }
}

#[derive(Default)]
pub struct HashMapThreadStore {
    threads: Arc<RwLock<HashMap<String, Thread>>>,
}

impl HashMapThreadStore {
    pub fn new() -> Self {
        Self {
            threads: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl ThreadStore for HashMapThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread> {
        let thread = Thread::new(request.agent_id, request.title, request.thread_id);

        if let Some(initial_message) = request.initial_message {
            // Here you might want to update the thread with the initial message
            // For now, we'll just create the thread
            let _ = initial_message;
        }

        let mut threads = self.threads.write().await;
        threads.insert(thread.id.clone(), thread.clone());
        Ok(thread)
    }

    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>> {
        let threads = self.threads.read().await;
        Ok(threads.get(thread_id).cloned())
    }

    async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> anyhow::Result<Thread> {
        let mut threads = self.threads.write().await;
        let thread = threads
            .get_mut(thread_id)
            .ok_or_else(|| anyhow::anyhow!("Thread not found"))?;

        if let Some(title) = request.title {
            thread.title = title;
        }

        if let Some(metadata) = request.metadata {
            thread.metadata.extend(metadata);
        }

        thread.updated_at = chrono::Utc::now();
        Ok(thread.clone())
    }

    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut threads = self.threads.write().await;
        threads.remove(thread_id);
        Ok(())
    }

    async fn list_threads(
        &self,
        agent_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadSummary>> {
        let threads = self.threads.read().await;
        let mut summaries: Vec<ThreadSummary> = threads
            .values()
            .filter(|thread| {
                if let Some(agent_id) = agent_id {
                    thread.agent_id == agent_id
                } else {
                    true
                }
            })
            .map(|thread| ThreadSummary {
                id: thread.id.clone(),
                title: thread.title.clone(),
                agent_id: thread.agent_id.clone(),
                agent_name: thread.agent_id.clone(), // Assuming agent_name is same as agent_id
                updated_at: thread.updated_at,
                message_count: thread.message_count,
                last_message: thread.last_message.clone(),
            })
            .collect();

        // Sort by updated_at descending
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        // Apply offset and limit
        let offset = offset.unwrap_or(0) as usize;
        let limit = limit.unwrap_or(50) as usize;

        if offset < summaries.len() {
            summaries = summaries.into_iter().skip(offset).take(limit).collect();
        } else {
            summaries = vec![];
        }

        Ok(summaries)
    }

    async fn update_thread_with_message(
        &self,
        thread_id: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let mut threads = self.threads.write().await;
        if let Some(thread) = threads.get_mut(thread_id) {
            thread.update_with_message(message);
        }
        Ok(())
    }
}

pub struct InMemoryAgentStore {
    agents: Arc<RwLock<HashMap<String, Box<dyn crate::agent::BaseAgent>>>>,
}

impl InMemoryAgentStore {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn insert(&self, name: String, agent: Box<dyn crate::agent::BaseAgent>) {
        let mut agents = self.agents.write().await;
        agents.insert(name, agent);
    }
}

#[async_trait]
impl AgentStore for InMemoryAgentStore {
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<Box<dyn crate::agent::BaseAgent>>, Option<String>) {
        let agents = self.agents.read().await;
        let limit = limit.unwrap_or(100);

        let start_index = if let Some(cursor) = cursor {
            agents
                .keys()
                .enumerate()
                .find(|(_, name)| **name == cursor)
                .map(|(i, _)| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let agent_entries: Vec<_> = agents.iter().skip(start_index).take(limit).collect();
        let results: Vec<Box<dyn crate::agent::BaseAgent>> = agent_entries
            .iter()
            .map(|(_, agent)| agent.clone_box())
            .collect();

        let next_cursor = if agent_entries.len() == limit {
            agent_entries.last().map(|(name, _)| (*name).clone())
        } else {
            None
        };

        (results, next_cursor)
    }

    async fn get(&self, name: &str) -> Option<Box<dyn crate::agent::BaseAgent>> {
        let agents = self.agents.read().await;
        agents.get(name).map(|agent| agent.clone_box())
    }

    async fn register(&self, agent: Box<dyn crate::agent::BaseAgent>) -> anyhow::Result<()> {
        let mut agents = self.agents.write().await;
        agents.insert(agent.get_name().to_string(), agent);
        Ok(())
    }
}
