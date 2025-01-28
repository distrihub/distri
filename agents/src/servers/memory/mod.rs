use anyhow::Result;
use serde::{Deserialize, Serialize};
mod file_memory;
pub use file_memory::FileMemory;
pub mod build;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entity {
    pub name: String,
    pub entity_type: String,
    pub observations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub relation_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct KnowledgeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

#[async_trait::async_trait]
pub trait Memory: Send + Sync {
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<Vec<Entity>>;
    async fn create_relations(&self, relations: Vec<Relation>) -> Result<Vec<Relation>>;
    async fn add_observations(
        &self,
        observations: Vec<(String, Vec<String>)>,
    ) -> Result<Vec<(String, Vec<String>)>>;
    async fn delete_entities(&self, entity_names: Vec<String>) -> Result<()>;
    async fn delete_observations(&self, deletions: Vec<(String, Vec<String>)>) -> Result<()>;
    async fn delete_relations(&self, relations: Vec<Relation>) -> Result<()>;
    async fn read_graph(&self) -> Result<KnowledgeGraph>;
    async fn search_nodes(&self, query: &str) -> Result<KnowledgeGraph>;
    async fn open_nodes(&self, names: Vec<String>) -> Result<KnowledgeGraph>;
}
