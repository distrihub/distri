use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    agent::Agent,
    coordinator::CoordinatorContext,
    memory::{AgentMemory, LocalAgentMemory, MemoryStep},
    types::{
        CreateThreadRequest, McpSession, Message, ServerTools, Thread, ThreadSummary,
        UpdateThreadRequest,
    },
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

// File-based SessionStore implementation
#[derive(Clone)]
pub struct FileSessionStore {
    file_path: String,
    sessions: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
}

impl FileSessionStore {
    pub fn new(file_path: String) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));
        std::fs::create_dir_all(&file_path).unwrap_or_default();
        Self {
            file_path,
            sessions,
        }
    }

    fn get_file_path(&self, thread_id: &str) -> String {
        format!("{}/{}.session", self.file_path, thread_id)
    }

    async fn load_from_file(&self, thread_id: &str) -> anyhow::Result<()> {
        let path = self.get_file_path(thread_id);
        if !tokio::fs::try_exists(&path).await? {
            return Ok(());
        }

        let contents = tokio::fs::read_to_string(&path).await?;
        let memory: LocalAgentMemory = serde_json::from_str(&contents)?;

        let mut sessions = self.sessions.write().await;
        sessions.insert(thread_id.to_string(), memory);
        Ok(())
    }

    async fn save_to_file(&self, thread_id: &str) -> anyhow::Result<()> {
        let sessions = self.sessions.read().await;

        if let Some(memory) = sessions.get(thread_id) {
            let serialized = serde_json::to_string(memory)?;
            let path = self.get_file_path(thread_id);
            tokio::fs::write(&path, serialized).await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl SessionStore for FileSessionStore {
    async fn get_steps(&self, thread_id: &str) -> anyhow::Result<Vec<MemoryStep>> {
        self.load_from_file(thread_id).await?;
        let sessions = self.sessions.read().await;
        let memory = sessions
            .get(thread_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::default);
        Ok(memory.get_steps())
    }

    async fn store_step(&self, thread_id: &str, step: MemoryStep) -> anyhow::Result<()> {
        {
            let mut sessions = self.sessions.write().await;
            let memory = sessions
                .entry(thread_id.to_string())
                .or_insert_with(|| LocalAgentMemory::new(thread_id.to_string()));
            memory.add_step(step);
        }
        self.save_to_file(thread_id).await?;
        Ok(())
    }

    async fn clear_session(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(thread_id);

        let path = self.get_file_path(thread_id);
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    async fn inc_iteration(&self, run_id: &str) -> anyhow::Result<i32> {
        let iteration = {
            let mut sessions = self.sessions.write().await;
            let memory = sessions
                .entry(run_id.to_string())
                .or_insert_with(|| LocalAgentMemory::new(run_id.to_string()));
            memory.iteration += 1;
            memory.iteration
        };
        self.save_to_file(run_id).await?;
        Ok(iteration)
    }

    async fn get_iteration(&self, run_id: &str) -> anyhow::Result<i32> {
        let sessions = self.sessions.read().await;
        let memory = sessions
            .get(run_id)
            .cloned()
            .unwrap_or_else(LocalAgentMemory::default);
        Ok(memory.iteration)
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

// File-based MemoryStore implementation using user_id
#[derive(Clone)]
pub struct FileMemoryStore {
    file_path: String,
    memories: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl FileMemoryStore {
    pub fn new(file_path: String) -> Self {
        std::fs::create_dir_all(&file_path).unwrap_or_default();
        Self {
            file_path,
            memories: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn get_memory_file_path(&self, user_id: &str) -> String {
        format!("{}/{}.memories", self.file_path, user_id)
    }

    async fn load_user_memories(&self, user_id: &str) -> anyhow::Result<()> {
        let path = self.get_memory_file_path(user_id);
        if !tokio::fs::try_exists(&path).await? {
            return Ok(());
        }

        let contents = tokio::fs::read_to_string(&path).await?;
        let user_memories: Vec<String> = serde_json::from_str(&contents)?;

        let mut memories = self.memories.write().await;
        memories.insert(user_id.to_string(), user_memories);
        Ok(())
    }

    async fn save_user_memories(&self, user_id: &str) -> anyhow::Result<()> {
        let memories = self.memories.read().await;
        if let Some(user_memories) = memories.get(user_id) {
            let serialized = serde_json::to_string(user_memories)?;
            let path = self.get_memory_file_path(user_id);
            tokio::fs::write(&path, serialized).await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl MemoryStore for FileMemoryStore {
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()> {
        self.load_user_memories(user_id).await?;

        {
            let mut memories = self.memories.write().await;
            let user_memories = memories.entry(user_id.to_string()).or_insert_with(Vec::new);

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
        }

        self.save_user_memories(user_id).await?;
        Ok(())
    }

    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>> {
        self.load_user_memories(user_id).await?;

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
        self.load_user_memories(user_id).await?;
        let memories = self.memories.read().await;
        Ok(memories.get(user_id).cloned().unwrap_or_default())
    }

    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()> {
        let mut memories = self.memories.write().await;
        memories.remove(user_id);

        let path = self.get_memory_file_path(user_id);
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}

// Task Store trait for A2A task management
#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn create_task(
        &self,
        context_id: &str,
        kind: &str,
        task_id: Option<&str>,
    ) -> anyhow::Result<Task>;
    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;
    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;
    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()>;
    async fn list_tasks(&self, thread_id: Option<&str>) -> anyhow::Result<Vec<Task>>;
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
        context_id: &str,
        kind: &str,
        task_id: Option<&str>,
    ) -> anyhow::Result<Task> {
        let task_id = task_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
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

    async fn list_tasks(&self, context_id: Option<&str>) -> anyhow::Result<Vec<Task>> {
        let tasks = self.tasks.read().await;
        let tasks = tasks
            .values()
            .filter(|task| context_id.map_or(true, |cid| task.context_id == cid))
            .cloned()
            .collect();
        Ok(tasks)
    }
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

// HashMap-based thread store implementation
#[derive(Clone, Default)]
pub struct HashMapThreadStore {
    threads: Arc<RwLock<HashMap<String, Thread>>>,
}

#[async_trait]
impl ThreadStore for HashMapThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread> {
        let mut thread = Thread::new(request.agent_id, request.title, request.thread_id);

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

    async fn list_threads(
        &self,
        agent_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadSummary>> {
        let threads = self.threads.read().await;

        let mut thread_list: Vec<ThreadSummary> = threads
            .values()
            .filter(|thread| agent_id.map_or(true, |aid| thread.agent_id == aid))
            .map(|thread| ThreadSummary {
                id: thread.id.clone(),
                title: thread.title.clone(),
                agent_id: thread.agent_id.clone(),
                agent_name: thread.agent_id.clone(),
                updated_at: thread.updated_at,
                message_count: thread.message_count,
                last_message: thread.last_message.clone(),
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

#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Returns a page of agents and an optional next cursor
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<Agent>, Option<String>);
    async fn get(&self, name: &str) -> Option<Agent>;
    async fn get_tools(&self, name: &str) -> Option<Vec<ServerTools>>;
    async fn register(&self, agent: Agent, tools: Vec<ServerTools>) -> anyhow::Result<()>;
}

#[derive(Clone, Default)]
pub struct InMemoryAgentStore {
    agents: Arc<RwLock<HashMap<String, Agent>>>,
    agent_tools: Arc<RwLock<HashMap<String, Vec<ServerTools>>>>,
}

impl InMemoryAgentStore {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            agent_tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub async fn insert(&self, name: String, agent: Agent) {
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
    ) -> (Vec<Agent>, Option<String>) {
        let agents = self.agents.read().await;
        let mut keys: Vec<&String> = agents.keys().collect();
        keys.sort();

        let start = match cursor {
            Some(ref c) => keys.iter().position(|k| *k > c).unwrap_or(0),
            None => 0,
        };
        let limit = limit.unwrap_or(30);
        let mut result = Vec::new();
        let mut next_cursor = None;
        for k in keys.iter().skip(start).take(limit) {
            if let Some(agent) = agents.get(*k) {
                result.push(agent.clone());
            }
        }
        if start + limit < keys.len() {
            next_cursor = keys.get(start + limit).map(|k| (*k).clone());
        }
        (result, next_cursor)
    }
    async fn get(&self, name: &str) -> Option<Agent> {
        let agents = self.agents.read().await;
        agents.get(name).cloned()
    }
    async fn get_tools(&self, name: &str) -> Option<Vec<ServerTools>> {
        let agent_tools = self.agent_tools.read().await;
        agent_tools.get(name).cloned()
    }
    async fn register(&self, agent: Agent, tools: Vec<ServerTools>) -> anyhow::Result<()> {
        let name = agent.definition.name.clone();
        let mut agents = self.agents.write().await;
        agents.insert(name.clone(), agent);
        let mut agent_tools = self.agent_tools.write().await;
        agent_tools.insert(name, tools);
        Ok(())
    }
}
