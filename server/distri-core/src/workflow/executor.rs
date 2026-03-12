use crate::agent::AgentOrchestrator;
use anyhow::{anyhow, Result};

use distri_types::{stores::SessionStore, workflow::WorkflowResult};
use std::sync::Arc;

pub struct WorkflowExecutor {
    orchestrator: Arc<AgentOrchestrator>,
}

impl WorkflowExecutor {
    pub fn new(orchestrator: Arc<AgentOrchestrator>) -> Self {
        Self { orchestrator }
    }

    /// Execute a DAP workflow
    pub async fn execute(
        &self,
        _workflow_id: &str,
        _params: serde_json::Value,
    ) -> Result<WorkflowResult> {
        Err(anyhow!("DAP workflow execution not supported (plugin system removed)"))
    }

    pub fn get_session_store(&self) -> Arc<dyn SessionStore> {
        self.orchestrator.stores.session_store.clone()
    }
}

impl Clone for WorkflowExecutor {
    fn clone(&self) -> Self {
        Self {
            orchestrator: self.orchestrator.clone(),
        }
    }
}
