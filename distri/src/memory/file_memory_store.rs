use crate::{
    memory::{LocalAgentMemory, MemoryStep},
    store::MemoryStore,
};
use async_trait::async_trait;
use std::{collections::HashMap, io, sync::Arc};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{debug, info};

#[derive(Clone)]
pub struct FileMemoryStore {
    file_path: String,
    memories: Arc<RwLock<HashMap<String, LocalAgentMemory>>>,
}

impl FileMemoryStore {
    pub fn get_file_path(&self, agent_id: &str) -> String {
        format!("{}/{}.memory", self.file_path, agent_id)
    }

    pub fn new(file_path: String) -> Self {
        let memories = Arc::new(RwLock::new(HashMap::new()));

        std::fs::create_dir_all(&file_path).unwrap_or_default();
        Self {
            file_path,
            memories,
        }
    }

    async fn load_from_file(&self, agent_id: &str) -> io::Result<()> {
        let contents = {
            let path = self.get_file_path(agent_id);
            info!("Loading memories from file: {}", path);
            tokio::fs::read_to_string(&path).await?
        };
        let mut memories = self.memories.write().await;
        *memories = deserialize_memories(&contents);
        info!("Successfully loaded memories for agent: {}", agent_id);
        Ok(())
    }

    async fn save_to_file(&self, agent_id: &str) -> io::Result<()> {
        let serialized = {
            let memories = self.memories.read().await;
            serialize_memories(&memories)
        };

        let path = self.get_file_path(agent_id);
        debug!("Saving memories to file: {}", path);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await?;
        file.write_all(serialized.as_bytes()).await?;
        debug!("Successfully saved memories for agent: {}", agent_id);
        Ok(())
    }
}

#[async_trait]
impl MemoryStore for FileMemoryStore {
    async fn get_steps(
        &self,
        agent_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryStep>> {
        debug!("Getting steps for agent: {}", agent_id);
        self.load_from_file(agent_id).await?;
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
        info!("Storing step for agent: {}", agent_id);
        {
            let mut memories = self.memories.write().await;
            let memory = memories
                .entry(agent_id.to_string())
                .or_insert_with(LocalAgentMemory::default);
            memory.add_step(step, thread_id);
        }

        self.save_to_file(agent_id).await?;
        Ok(())
    }
}

fn serialize_memories(memories: &HashMap<String, LocalAgentMemory>) -> String {
    serde_json::to_string(memories).unwrap_or_else(|_| String::new())
}

fn deserialize_memories(contents: &str) -> HashMap<String, LocalAgentMemory> {
    serde_json::from_str(contents).unwrap_or_else(|_| HashMap::new())
}
