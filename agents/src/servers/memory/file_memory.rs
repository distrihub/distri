use super::{Entity, KnowledgeGraph, Memory, Relation};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;

pub struct FileMemory {
    file_path: PathBuf,
    graph: Arc<Mutex<KnowledgeGraph>>,
}

impl FileMemory {
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file_path = PathBuf::from(path.as_ref());
        let graph = match fs::read_to_string(&file_path).await {
            Ok(contents) => {
                let lines: Vec<_> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
                let mut graph = KnowledgeGraph::default();

                for line in lines {
                    let value: Value = serde_json::from_str(line)?;
                    match value.get("type").and_then(Value::as_str) {
                        Some("entity") => {
                            let entity: Entity = serde_json::from_value(value)?;
                            graph.entities.push(entity);
                        }
                        Some("relation") => {
                            let relation: Relation = serde_json::from_value(value)?;
                            graph.relations.push(relation);
                        }
                        _ => continue,
                    }
                }
                graph
            }
            Err(_) => KnowledgeGraph::default(),
        };

        Ok(Self {
            file_path,
            graph: Arc::new(Mutex::new(graph)),
        })
    }

    async fn save_graph(&self, graph: &KnowledgeGraph) -> Result<()> {
        let mut contents = String::new();

        for entity in &graph.entities {
            let value = json!({
                "type": "entity",
                "name": entity.name,
                "entity_type": entity.entity_type,
                "observations": entity.observations
            });
            contents.push_str(&serde_json::to_string(&value)?);
            contents.push('\n');
        }

        for relation in &graph.relations {
            let value = json!({
                "type": "relation",
                "from": relation.from,
                "to": relation.to,
                "relation_type": relation.relation_type
            });
            contents.push_str(&serde_json::to_string(&value)?);
            contents.push('\n');
        }

        fs::write(&self.file_path, contents).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Memory for FileMemory {
    async fn create_entities(&self, entities: Vec<Entity>) -> Result<Vec<Entity>> {
        let mut graph = self.graph.lock().await;
        let new_entities: Vec<_> = entities
            .into_iter()
            .filter(|e| {
                !graph
                    .entities
                    .iter()
                    .any(|existing| existing.name == e.name)
            })
            .collect();

        graph.entities.extend(new_entities.clone());
        self.save_graph(&graph).await?;
        Ok(new_entities)
    }

    async fn create_relations(&self, relations: Vec<Relation>) -> Result<Vec<Relation>> {
        let mut graph = self.graph.lock().await;
        let new_relations: Vec<_> = relations
            .into_iter()
            .filter(|r| {
                !graph.relations.iter().any(|existing| {
                    existing.from == r.from
                        && existing.to == r.to
                        && existing.relation_type == r.relation_type
                })
            })
            .collect();

        graph.relations.extend(new_relations.clone());
        self.save_graph(&graph).await?;
        Ok(new_relations)
    }

    async fn add_observations(
        &self,
        observations: Vec<(String, Vec<String>)>,
    ) -> Result<Vec<(String, Vec<String>)>> {
        let mut graph = self.graph.lock().await;
        let mut added = Vec::new();

        for (entity_name, contents) in observations {
            if let Some(entity) = graph.entities.iter_mut().find(|e| e.name == entity_name) {
                let new_obs: Vec<_> = contents
                    .into_iter()
                    .filter(|c| !entity.observations.contains(c))
                    .collect();
                entity.observations.extend(new_obs.clone());
                added.push((entity_name, new_obs));
            }
        }

        self.save_graph(&graph).await?;
        Ok(added)
    }

    async fn delete_entities(&self, entity_names: Vec<String>) -> Result<()> {
        let mut graph = self.graph.lock().await;
        graph.entities.retain(|e| !entity_names.contains(&e.name));
        graph
            .relations
            .retain(|r| !entity_names.contains(&r.from) && !entity_names.contains(&r.to));
        self.save_graph(&graph).await
    }

    async fn delete_observations(&self, deletions: Vec<(String, Vec<String>)>) -> Result<()> {
        let mut graph = self.graph.lock().await;
        for (entity_name, to_delete) in deletions {
            if let Some(entity) = graph.entities.iter_mut().find(|e| e.name == entity_name) {
                entity.observations.retain(|o| !to_delete.contains(o));
            }
        }
        self.save_graph(&graph).await
    }

    async fn delete_relations(&self, relations: Vec<Relation>) -> Result<()> {
        let mut graph = self.graph.lock().await;
        graph.relations.retain(|r| {
            !relations.iter().any(|del| {
                del.from == r.from && del.to == r.to && del.relation_type == r.relation_type
            })
        });
        self.save_graph(&graph).await
    }

    async fn read_graph(&self) -> Result<KnowledgeGraph> {
        Ok(self.graph.lock().await.clone())
    }

    async fn search_nodes(&self, query: &str) -> Result<KnowledgeGraph> {
        let graph = self.graph.lock().await;
        let query = query.to_lowercase();

        let entities: Vec<_> = graph
            .entities
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&query)
                    || e.entity_type.to_lowercase().contains(&query)
                    || e.observations
                        .iter()
                        .any(|o| o.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();

        let entity_names: std::collections::HashSet<_> = entities.iter().map(|e| &e.name).collect();
        let relations: Vec<_> = graph
            .relations
            .iter()
            .filter(|r| entity_names.contains(&r.from) && entity_names.contains(&r.to))
            .cloned()
            .collect();

        Ok(KnowledgeGraph {
            entities,
            relations,
        })
    }

    async fn open_nodes(&self, names: Vec<String>) -> Result<KnowledgeGraph> {
        let graph = self.graph.lock().await;
        let entities: Vec<_> = graph
            .entities
            .iter()
            .filter(|e| names.contains(&e.name))
            .cloned()
            .collect();

        let entity_names: std::collections::HashSet<_> = entities.iter().map(|e| &e.name).collect();
        let relations: Vec<_> = graph
            .relations
            .iter()
            .filter(|r| entity_names.contains(&r.from) && entity_names.contains(&r.to))
            .cloned()
            .collect();

        Ok(KnowledgeGraph {
            entities,
            relations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    async fn create_test_memory() -> Result<(FileMemory, NamedTempFile)> {
        let temp_file = NamedTempFile::new()?;
        let memory = FileMemory::new(temp_file.path()).await?;
        Ok((memory, temp_file))
    }

    #[tokio::test]
    async fn test_create_entities() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        let entities = vec![
            Entity {
                name: "Alice".to_string(),
                entity_type: "person".to_string(),
                observations: vec!["likes coffee".to_string()],
            },
            Entity {
                name: "Bob".to_string(),
                entity_type: "person".to_string(),
                observations: vec!["likes tea".to_string()],
            },
        ];

        let created = memory.create_entities(entities.clone()).await?;
        assert_eq!(created.len(), 2);

        // Test duplicate creation
        let duplicates = memory.create_entities(entities).await?;
        assert_eq!(duplicates.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_create_relations() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // First create some entities
        let entities = vec![
            Entity {
                name: "Alice".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
            Entity {
                name: "Bob".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
        ];
        memory.create_entities(entities).await?;

        // Create relations
        let relations = vec![Relation {
            from: "Alice".to_string(),
            to: "Bob".to_string(),
            relation_type: "friend".to_string(),
        }];

        let created = memory.create_relations(relations.clone()).await?;
        assert_eq!(created.len(), 1);

        // Test duplicate creation
        let duplicates = memory.create_relations(relations).await?;
        assert_eq!(duplicates.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_add_observations() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // Create an entity first
        let entities = vec![Entity {
            name: "Alice".to_string(),
            entity_type: "person".to_string(),
            observations: vec!["likes coffee".to_string()],
        }];
        memory.create_entities(entities).await?;

        // Add new observations
        let observations = vec![(
            "Alice".to_string(),
            vec!["works hard".to_string(), "likes coffee".to_string()], // One new, one duplicate
        )];

        let added = memory.add_observations(observations).await?;
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].1.len(), 1); // Only one new observation should be added

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_entities() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // Create entities and relations
        let entities = vec![
            Entity {
                name: "Alice".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
            Entity {
                name: "Bob".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
        ];
        memory.create_entities(entities).await?;

        let relations = vec![Relation {
            from: "Alice".to_string(),
            to: "Bob".to_string(),
            relation_type: "friend".to_string(),
        }];
        memory.create_relations(relations).await?;

        // Delete Alice
        memory.delete_entities(vec!["Alice".to_string()]).await?;

        let graph = memory.read_graph().await?;
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.relations.len(), 0); // Relation should be deleted

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_observations() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // Create entity with observations
        let entities = vec![Entity {
            name: "Alice".to_string(),
            entity_type: "person".to_string(),
            observations: vec!["likes coffee".to_string(), "works hard".to_string()],
        }];
        memory.create_entities(entities).await?;

        // Delete one observation
        let deletions = vec![("Alice".to_string(), vec!["likes coffee".to_string()])];
        memory.delete_observations(deletions).await?;

        let graph = memory.read_graph().await?;
        assert_eq!(graph.entities[0].observations.len(), 1);
        assert_eq!(graph.entities[0].observations[0], "works hard");

        Ok(())
    }

    #[tokio::test]
    async fn test_search_nodes() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // Create test data
        let entities = vec![
            Entity {
                name: "Alice".to_string(),
                entity_type: "person".to_string(),
                observations: vec!["likes coffee".to_string()],
            },
            Entity {
                name: "Bob".to_string(),
                entity_type: "person".to_string(),
                observations: vec!["likes tea".to_string()],
            },
        ];
        memory.create_entities(entities).await?;

        let relations = vec![Relation {
            from: "Alice".to_string(),
            to: "Bob".to_string(),
            relation_type: "friend".to_string(),
        }];
        memory.create_relations(relations).await?;

        // Test search
        let results = memory.search_nodes("coffee").await?;
        assert_eq!(results.entities.len(), 1);
        assert_eq!(results.entities[0].name, "Alice");

        // Test case insensitive search
        let results = memory.search_nodes("COFFEE").await?;
        assert_eq!(results.entities.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_open_nodes() -> Result<()> {
        let (memory, _temp) = create_test_memory().await?;

        // Create test data
        let entities = vec![
            Entity {
                name: "Alice".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
            Entity {
                name: "Bob".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
            Entity {
                name: "Charlie".to_string(),
                entity_type: "person".to_string(),
                observations: vec![],
            },
        ];
        memory.create_entities(entities).await?;

        let relations = vec![
            Relation {
                from: "Alice".to_string(),
                to: "Bob".to_string(),
                relation_type: "friend".to_string(),
            },
            Relation {
                from: "Bob".to_string(),
                to: "Charlie".to_string(),
                relation_type: "friend".to_string(),
            },
        ];
        memory.create_relations(relations).await?;

        // Test opening specific nodes
        let result = memory
            .open_nodes(vec!["Alice".to_string(), "Bob".to_string()])
            .await?;
        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.relations.len(), 1); // Only Alice-Bob relation

        Ok(())
    }
}
