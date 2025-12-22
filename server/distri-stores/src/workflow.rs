use async_trait::async_trait;
use distri_types::workflow::{Workflow, WorkflowEvent, WorkflowResult, WorkflowStore};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// In-memory implementation of WorkflowStore for development and testing
#[derive(Default)]
pub struct InMemoryWorkflowStore {
    workflows: Arc<RwLock<HashMap<String, Workflow>>>,
    events: Arc<RwLock<HashMap<String, WorkflowEvent>>>,
    results: Arc<RwLock<HashMap<String, WorkflowResult>>>,
}

impl InMemoryWorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl WorkflowStore for InMemoryWorkflowStore {
    async fn store_workflow(&self, workflow: Workflow) -> anyhow::Result<()> {
        let mut workflows = self.workflows.write().unwrap();
        workflows.insert(workflow.id.clone(), workflow);
        Ok(())
    }

    async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<Workflow>> {
        let workflows = self.workflows.read().unwrap();
        Ok(workflows.get(id).cloned())
    }

    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
        let workflows = self.workflows.read().unwrap();
        Ok(workflows.values().cloned().collect())
    }

    async fn update_workflow(&self, workflow: Workflow) -> anyhow::Result<()> {
        let mut workflows = self.workflows.write().unwrap();
        workflows.insert(workflow.id.clone(), workflow);
        Ok(())
    }

    async fn delete_workflow(&self, id: &str) -> anyhow::Result<()> {
        let mut workflows = self.workflows.write().unwrap();
        workflows.remove(id);
        Ok(())
    }

    async fn store_event(&self, event: WorkflowEvent) -> anyhow::Result<()> {
        let mut events = self.events.write().unwrap();
        events.insert(event.id.clone(), event);
        Ok(())
    }

    async fn get_event(&self, event_id: &str) -> anyhow::Result<Option<WorkflowEvent>> {
        let events = self.events.read().unwrap();
        Ok(events.get(event_id).cloned())
    }

    async fn store_result(&self, result: WorkflowResult) -> anyhow::Result<()> {
        let mut results = self.results.write().unwrap();
        results.insert(result.event_id.clone(), result);
        Ok(())
    }

    async fn get_result(&self, event_id: &str) -> anyhow::Result<Option<WorkflowResult>> {
        let results = self.results.read().unwrap();
        Ok(results.get(event_id).cloned())
    }

    async fn list_results(&self, workflow_id: &str) -> anyhow::Result<Vec<WorkflowResult>> {
        let results = self.results.read().unwrap();
        Ok(results
            .values()
            .filter(|r| r.workflow_id == workflow_id)
            .cloned()
            .collect())
    }
}
