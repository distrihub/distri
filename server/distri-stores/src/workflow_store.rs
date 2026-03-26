use async_trait::async_trait;
use chrono::Utc;
use distri_types::stores::{
    NewWorkflow, UpdateWorkflow, WorkflowFilter, WorkflowRecord, WorkflowStore,
};
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory workflow store for testing and OSS single-tenant use.
pub struct InMemoryWorkflowStore {
    workflows: Mutex<HashMap<String, WorkflowRecord>>,
}

impl InMemoryWorkflowStore {
    pub fn new() -> Self {
        Self {
            workflows: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl WorkflowStore for InMemoryWorkflowStore {
    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
    ) -> anyhow::Result<Vec<WorkflowRecord>> {
        let map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut results: Vec<WorkflowRecord> = map
            .values()
            .filter(|w| {
                if let Some(is_pub) = filter.is_public {
                    if w.is_public != is_pub {
                        return false;
                    }
                }
                if let Some(is_tpl) = filter.is_template {
                    if w.is_template != is_tpl {
                        return false;
                    }
                }
                if let Some(ref search) = filter.search {
                    let s = search.to_lowercase();
                    if !w.name.to_lowercase().contains(&s)
                        && !w
                            .description
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&s))
                            .unwrap_or(false)
                    {
                        return false;
                    }
                }
                if let Some(ref tags) = filter.tags {
                    if !tags.iter().any(|t| w.tags.contains(t)) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        results.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        let offset = filter.offset.unwrap_or(0) as usize;
        let limit = filter.limit.unwrap_or(100) as usize;
        let results = results.into_iter().skip(offset).take(limit).collect();

        Ok(results)
    }

    async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<WorkflowRecord>> {
        let map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(map.get(id).cloned())
    }

    async fn create_workflow(&self, workflow: NewWorkflow) -> anyhow::Result<WorkflowRecord> {
        let now = Utc::now();
        let record = WorkflowRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: workflow.name,
            description: workflow.description,
            definition: workflow.definition,
            tags: workflow.tags,
            is_public: workflow.is_public,
            is_template: workflow.is_template,
            star_count: 0,
            clone_count: 0,
            created_at: now,
            updated_at: now,
        };

        let mut map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        map.insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn update_workflow(
        &self,
        id: &str,
        update: UpdateWorkflow,
    ) -> anyhow::Result<WorkflowRecord> {
        let mut map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let record = map
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("Workflow not found"))?;

        if let Some(name) = update.name {
            record.name = name;
        }
        if let Some(desc) = update.description {
            record.description = Some(desc);
        }
        if let Some(def) = update.definition {
            record.definition = def;
        }
        if let Some(tags) = update.tags {
            record.tags = tags;
        }
        if let Some(is_pub) = update.is_public {
            record.is_public = is_pub;
        }
        record.updated_at = Utc::now();

        Ok(record.clone())
    }

    async fn delete_workflow(&self, id: &str) -> anyhow::Result<()> {
        let mut map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        map.remove(id)
            .ok_or_else(|| anyhow::anyhow!("Workflow not found"))?;
        Ok(())
    }

    async fn list_public_workflows(&self) -> anyhow::Result<Vec<WorkflowRecord>> {
        let map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(map
            .values()
            .filter(|w| w.is_public && w.is_template)
            .cloned()
            .collect())
    }

    async fn star_workflow(&self, _workflow_id: &str) -> anyhow::Result<()> {
        // No-op for in-memory (no user context)
        Ok(())
    }

    async fn unstar_workflow(&self, _workflow_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn list_starred_workflows(&self) -> anyhow::Result<Vec<WorkflowRecord>> {
        Ok(vec![])
    }

    async fn clone_workflow(&self, workflow_id: &str) -> anyhow::Result<WorkflowRecord> {
        let source = {
            let map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            map.get(workflow_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Workflow not found"))?
        };

        let now = Utc::now();
        let cloned = WorkflowRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("{} (copy)", source.name),
            description: source.description,
            definition: source.definition,
            tags: source.tags,
            is_public: false,
            is_template: false,
            star_count: 0,
            clone_count: 0,
            created_at: now,
            updated_at: now,
        };

        // Increment source clone count
        {
            let mut map = self.workflows.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            if let Some(src) = map.get_mut(workflow_id) {
                src.clone_count += 1;
            }
            map.insert(cloned.id.clone(), cloned.clone());
        }

        Ok(cloned)
    }
}
