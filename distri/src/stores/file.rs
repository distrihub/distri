use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{
    memory::{AgentMemory, LocalAgentMemory, MemoryStep},
    MemoryStore, SessionMemory, SessionStore,
};

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
                "Session: {} | Thread: {} | Summary: {} | Key Insights: {} | Important Facts: {}",
                session_memory.session_id,
                session_memory.user_id,
                session_memory.content,
                session_memory.metadata.get("key_insights").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("; ")).unwrap_or_default(),
                session_memory.metadata.get("important_facts").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("; ")).unwrap_or_default()
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
