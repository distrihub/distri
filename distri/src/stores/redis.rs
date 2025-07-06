#[cfg(feature = "redis")]
use async_trait::async_trait;
#[cfg(feature = "redis")]
use redis::{AsyncCommands, Client};
#[cfg(feature = "redis")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "redis")]
use std::{collections::HashMap, sync::Arc};
#[cfg(feature = "redis")]
use uuid::Uuid;

#[cfg(feature = "redis")]
use crate::{
    agent::ExecutorContext,
    memory::{LocalAgentMemory, MemoryStep},
    stores::{
        AgentStore, MemoryStore, SessionMemory, SessionStore, TaskStore, ThreadStore,
        ToolSessionStore,
    },
    types::{CreateThreadRequest, McpSession, Thread, ThreadSummary, UpdateThreadRequest},
};
#[cfg(feature = "redis")]
use distri_a2a::{Artifact, Message as A2aMessage, Task, TaskState, TaskStatus};

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisSessionStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisSessionStore {
    pub fn new(redis_url: &str, prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: prefix.unwrap_or_else(|| "distri:session".to_string()),
        })
    }

    fn session_key(&self, thread_id: &str) -> String {
        format!("{}:{}", self.prefix, thread_id)
    }

    fn iteration_key(&self, thread_id: &str) -> String {
        format!("{}:iterations:{}", self.prefix, thread_id)
    }
}

#[cfg(feature = "redis")]
#[async_trait::async_trait]
impl SessionStore for RedisSessionStore {
    async fn get_steps(&self, thread_id: &str) -> anyhow::Result<Vec<MemoryStep>> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.session_key(thread_id);

        let data: Option<String> = conn.get(&key).await?;
        match data {
            Some(json_data) => {
                let memory: LocalAgentMemory = serde_json::from_str(&json_data)?;
                Ok(memory.get_steps())
            }
            None => Ok(vec![]),
        }
    }

    async fn store_step(&self, thread_id: &str, step: MemoryStep) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.session_key(thread_id);

        // Get existing memory or create new
        let mut memory = match conn.get::<_, Option<String>>(&key).await? {
            Some(json_data) => serde_json::from_str::<LocalAgentMemory>(&json_data)?,
            None => LocalAgentMemory::new(thread_id.to_string()),
        };

        memory.add_step(step);
        let serialized = serde_json::to_string(&memory)?;
        conn.set(&key, serialized).await?;

        Ok(())
    }

    async fn clear_session(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.session_key(thread_id);
        let iteration_key = self.iteration_key(thread_id);

        conn.del(&[&key, &iteration_key]).await?;
        Ok(())
    }

    async fn inc_iteration(&self, thread_id: &str) -> anyhow::Result<i32> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.iteration_key(thread_id);

        let count: i32 = conn.incr(&key, 1).await?;
        Ok(count)
    }

    async fn get_iteration(&self, thread_id: &str) -> anyhow::Result<i32> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.iteration_key(thread_id);

        let count: Option<i32> = conn.get(&key).await?;
        Ok(count.unwrap_or(0))
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisMemoryStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisMemoryStore {
    pub fn new(redis_url: &str, prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: prefix.unwrap_or_else(|| "distri:memory".to_string()),
        })
    }

    fn memory_key(&self, user_id: &str) -> String {
        format!("{}:{}", self.prefix, user_id)
    }
}

#[cfg(feature = "redis")]
#[async_trait::async_trait]
impl MemoryStore for RedisMemoryStore {
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.memory_key(user_id);

        let memory_entry = format!(
            "Agent: {} | Session: {} ({})\nSummary: {}\nInsights: {}\nFacts: {}",
            session_memory.agent_id,
            session_memory.thread_id,
            session_memory.timestamp.format("%Y-%m-%d %H:%M:%S"),
            session_memory.session_summary,
            session_memory.key_insights.join("; "),
            session_memory.important_facts.join("; ")
        );

        conn.lpush(&key, &memory_entry).await?;
        Ok(())
    }

    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.memory_key(user_id);

        let memories: Vec<String> = conn.lrange(&key, 0, -1).await?;
        let query_lower = query.to_lowercase();
        let mut relevant_memories: Vec<String> = memories
            .into_iter()
            .filter(|memory| memory.to_lowercase().contains(&query_lower))
            .collect();

        if let Some(limit) = limit {
            relevant_memories.truncate(limit);
        }

        Ok(relevant_memories)
    }

    async fn get_user_memories(&self, user_id: &str) -> anyhow::Result<Vec<String>> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.memory_key(user_id);

        let memories: Vec<String> = conn.lrange(&key, 0, -1).await?;
        Ok(memories)
    }

    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.memory_key(user_id);

        conn.del(&key).await?;
        Ok(())
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisTaskStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisTaskStore {
    pub fn new(redis_url: &str, prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: prefix.unwrap_or_else(|| "distri:task".to_string()),
        })
    }

    fn task_key(&self, task_id: &str) -> String {
        format!("{}:{}", self.prefix, task_id)
    }

    fn context_tasks_key(&self, context_id: &str) -> String {
        format!("{}:context:{}", self.prefix, context_id)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl TaskStore for RedisTaskStore {
    async fn create_task(&self, context_id: &str, task_id: Option<&str>) -> anyhow::Result<Task> {
        let mut conn = self.client.get_async_connection().await?;
        let task_id = task_id.unwrap_or(&Uuid::new_v4().to_string()).to_string();

        let task = Task {
            id: task_id.clone(),
            context_id: context_id.to_string(),
            status: TaskStatus {
                state: TaskState::Created,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
            messages: vec![],
        };

        let serialized = serde_json::to_string(&task)?;
        let task_key = self.task_key(&task_id);
        let context_key = self.context_tasks_key(context_id);

        conn.set(&task_key, &serialized).await?;
        conn.sadd(&context_key, &task_id).await?;

        Ok(task)
    }

    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.task_key(task_id);

        let data: Option<String> = conn.get(&key).await?;
        match data {
            Some(json_data) => {
                let task: Task = serde_json::from_str(&json_data)?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.task_key(task_id);

        if let Some(mut task) = self.get_task(task_id).await? {
            task.status = status;
            let serialized = serde_json::to_string(&task)?;
            conn.set(&key, serialized).await?;
        }

        Ok(())
    }

    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.task_key(task_id);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        task.status = TaskStatus {
            state: TaskState::Cancelled,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        };

        let serialized = serde_json::to_string(&task)?;
        conn.set(&key, serialized).await?;

        Ok(task)
    }

    async fn add_message_to_task(&self, task_id: &str, message: A2aMessage) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.task_key(task_id);

        if let Some(mut task) = self.get_task(task_id).await? {
            task.messages.push(message);
            let serialized = serde_json::to_string(&task)?;
            conn.set(&key, serialized).await?;
        }

        Ok(())
    }

    async fn add_artifact_to_task(&self, task_id: &str, artifact: Artifact) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.task_key(task_id);

        if let Some(mut task) = self.get_task(task_id).await? {
            task.artifacts.push(artifact);
            let serialized = serde_json::to_string(&task)?;
            conn.set(&key, serialized).await?;
        }

        Ok(())
    }

    async fn list_tasks(&self, context_id: Option<&str>) -> anyhow::Result<Vec<Task>> {
        let mut conn = self.client.get_async_connection().await?;

        if let Some(context_id) = context_id {
            let context_key = self.context_tasks_key(context_id);
            let task_ids: Vec<String> = conn.smembers(&context_key).await?;

            let mut tasks = Vec::new();
            for task_id in task_ids {
                if let Some(task) = self.get_task(&task_id).await? {
                    tasks.push(task);
                }
            }
            Ok(tasks)
        } else {
            // This is inefficient for Redis - in production, you'd want to maintain a global index
            // For now, return empty as this is a fallback case
            Ok(vec![])
        }
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisThreadStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisThreadStore {
    pub fn new(redis_url: &str, prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: prefix.unwrap_or_else(|| "distri:thread".to_string()),
        })
    }

    fn thread_key(&self, thread_id: &str) -> String {
        format!("{}:{}", self.prefix, thread_id)
    }

    fn agent_threads_key(&self, agent_id: &str) -> String {
        format!("{}:agent:{}", self.prefix, agent_id)
    }

    fn all_threads_key(&self) -> String {
        format!("{}:all", self.prefix)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl ThreadStore for RedisThreadStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread> {
        let mut conn = self.client.get_async_connection().await?;

        let thread = Thread::new(request.agent_id.clone(), request.title, request.thread_id);

        let serialized = serde_json::to_string(&thread)?;
        let thread_key = self.thread_key(&thread.id);
        let agent_key = self.agent_threads_key(&request.agent_id);
        let all_key = self.all_threads_key();

        conn.set(&thread_key, &serialized).await?;
        conn.sadd(&agent_key, &thread.id).await?;
        conn.sadd(&all_key, &thread.id).await?;

        Ok(thread)
    }

    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.thread_key(thread_id);

        let data: Option<String> = conn.get(&key).await?;
        match data {
            Some(json_data) => {
                let thread: Thread = serde_json::from_str(&json_data)?;
                Ok(Some(thread))
            }
            None => Ok(None),
        }
    }

    async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> anyhow::Result<Thread> {
        let mut conn = self.client.get_async_connection().await?;
        let key = self.thread_key(thread_id);

        let mut thread = self
            .get_thread(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Thread not found"))?;

        if let Some(title) = request.title {
            thread.title = title;
        }

        if let Some(metadata) = request.metadata {
            thread.metadata.extend(metadata);
        }

        thread.updated_at = chrono::Utc::now();

        let serialized = serde_json::to_string(&thread)?;
        conn.set(&key, serialized).await?;

        Ok(thread)
    }

    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;

        if let Some(thread) = self.get_thread(thread_id).await? {
            let thread_key = self.thread_key(thread_id);
            let agent_key = self.agent_threads_key(&thread.agent_id);
            let all_key = self.all_threads_key();

            conn.del(&thread_key).await?;
            conn.srem(&agent_key, thread_id).await?;
            conn.srem(&all_key, thread_id).await?;
        }

        Ok(())
    }

    async fn list_threads(
        &self,
        agent_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<Vec<ThreadSummary>> {
        let mut conn = self.client.get_async_connection().await?;

        let thread_ids: Vec<String> = if let Some(agent_id) = agent_id {
            let agent_key = self.agent_threads_key(agent_id);
            conn.smembers(&agent_key).await?
        } else {
            let all_key = self.all_threads_key();
            conn.smembers(&all_key).await?
        };

        let mut summaries = Vec::new();
        for thread_id in thread_ids {
            if let Some(thread) = self.get_thread(&thread_id).await? {
                summaries.push(ThreadSummary {
                    id: thread.id,
                    title: thread.title,
                    agent_id: thread.agent_id.clone(),
                    agent_name: thread.agent_id, // Assuming agent_name is same as agent_id
                    updated_at: thread.updated_at,
                    message_count: thread.message_count,
                    last_message: thread.last_message,
                });
            }
        }

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
        let mut conn = self.client.get_async_connection().await?;
        let key = self.thread_key(thread_id);

        if let Some(mut thread) = self.get_thread(thread_id).await? {
            thread.update_with_message(message);
            let serialized = serde_json::to_string(&thread)?;
            conn.set(&key, serialized).await?;
        }

        Ok(())
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisToolSessionStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisToolSessionStore {
    pub fn new(redis_url: &str, prefix: Option<String>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: prefix.unwrap_or_else(|| "distri:tool_session".to_string()),
        })
    }

    fn session_key(&self, server_name: &str, user_id: &str) -> String {
        format!("{}:{}:{}", self.prefix, server_name, user_id)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl ToolSessionStore for RedisToolSessionStore {
    async fn get_session(
        &self,
        server_name: &str,
        context: &ExecutorContext,
    ) -> anyhow::Result<Option<McpSession>> {
        let mut conn = self.client.get_async_connection().await?;
        let user_id = context.user_id.as_deref().unwrap_or("default");
        let key = self.session_key(server_name, user_id);

        let data: Option<String> = conn.get(&key).await?;
        match data {
            Some(json_data) => {
                let session: McpSession = serde_json::from_str(&json_data)?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisAgentStore {
    client: Client,
    prefix: String,
}

#[cfg(feature = "redis")]
impl RedisAgentStore {
    pub async fn new(redis_url: &str) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client,
            prefix: "distri:agent".to_string(),
        })
    }

    fn agent_key(&self, name: &str) -> String {
        format!("{}:{}", self.prefix, name)
    }

    fn agents_list_key(&self) -> String {
        format!("{}:list", self.prefix)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl AgentStore for RedisAgentStore {
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<Box<dyn crate::agent::BaseAgent>>, Option<String>) {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(_) => return (Vec::new(), None),
        };

        let limit = limit.unwrap_or(100);
        let start_index = cursor.unwrap_or_else(|| "0".to_string());

        // Get agent names from sorted set
        let agent_names: Vec<String> = match conn
            .zrange(&self.agents_list_key(), &start_index, limit as isize - 1)
            .await
        {
            Ok(names) => names,
            Err(_) => return (Vec::new(), None),
        };

        let mut agents = Vec::new();
        for name in &agent_names {
            if let Ok(Some(serialized)) = conn.get::<_, Option<String>>(&self.agent_key(name)).await
            {
                if let Ok(definition) =
                    serde_json::from_str::<crate::types::AgentDefinition>(&serialized)
                {
                    // Note: We need to reconstruct the agent from stored definition
                    // In practice, you might store the entire agent or have a way to reconstruct it
                    // For now, we'll return empty agents vector as this is complex to implement properly
                    // without access to the full agent creation context
                }
            }
        }

        let next_cursor = if agent_names.len() == limit {
            agent_names.last().cloned()
        } else {
            None
        };

        (agents, next_cursor)
    }

    async fn get(&self, name: &str) -> Option<Box<dyn crate::agent::BaseAgent>> {
        let mut conn = self.client.get_async_connection().await.ok()?;
        let key = self.agent_key(name);

        let serialized: Option<String> = conn.get(&key).await.ok()?;
        // Note: Similar issue as with list - we'd need to reconstruct the agent
        // This is a limitation of storing agents in Redis without full context
        None
    }

    async fn register(&self, agent: Box<dyn crate::agent::BaseAgent>) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let name = agent.get_name();
        let definition = agent.get_definition();

        let serialized = serde_json::to_string(&definition)?;
        let agent_key = self.agent_key(name);

        // Store agent definition
        conn.set(&agent_key, &serialized).await?;

        // Add to agents list
        conn.zadd(&self.agents_list_key(), name, 0).await?;

        Ok(())
    }

    async fn update(&self, agent: Box<dyn crate::agent::BaseAgent>) -> anyhow::Result<()> {
        let mut conn = self.client.get_async_connection().await?;
        let name = agent.get_name();
        let agent_key = self.agent_key(name);

        // Check if agent exists
        let exists: bool = conn.exists(&agent_key).await?;
        if !exists {
            return Err(anyhow::anyhow!("Agent '{}' not found", name));
        }

        let definition = agent.get_definition();
        let serialized = serde_json::to_string(&definition)?;

        // Update agent definition
        conn.set(&agent_key, &serialized).await?;

        Ok(())
    }
}
