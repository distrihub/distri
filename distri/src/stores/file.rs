use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use crate::{LocalSession, MemoryStore, SessionMemory, SessionStore};

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

// File-based SessionStore implementation
#[derive(Clone)]
pub struct FileSessionStore {
    file_path: String,
    sessions: Arc<RwLock<HashMap<String, LocalSession>>>,
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
    #[allow(dead_code)]
    async fn load_from_file(&self, thread_id: &str) -> anyhow::Result<()> {
        let path = self.get_file_path(thread_id);
        if !tokio::fs::try_exists(&path).await? {
            return Ok(());
        }

        let contents = tokio::fs::read_to_string(&path).await?;
        let memory: LocalSession = serde_json::from_str(&contents)?;

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
    async fn set_value(&self, thread_id: &str, key: &str, value: &str) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .entry(thread_id.to_string())
            .or_insert_with(LocalSession::default);
        session.values.insert(key.to_string(), value.to_string());
        self.save_to_file(thread_id).await?;
        Ok(())
    }

    async fn get_value(&self, thread_id: &str, key: &str) -> anyhow::Result<Option<String>> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(thread_id).cloned().unwrap_or_default();
        Ok(session.values.get(key).cloned())
    }

    async fn delete_value(&self, thread_id: &str, key: &str) -> anyhow::Result<()> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(thread_id).unwrap();
        session.values.remove(key);
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
}
