use crate::agent::{AgentOrchestrator, ExecutorContext};
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

    /// Execute a DAP workflow using the new plugin system
    pub async fn execute(
        &self,
        workflow_id: &str,
        params: serde_json::Value,
        dap_registry: Arc<tokio::sync::RwLock<crate::agent::plugin_registry::PluginRegistry>>,
        context: Arc<ExecutorContext>,
    ) -> Result<WorkflowResult> {
        use chrono::Utc;
        use distri_plugin_executor::{PluginContext, PluginExecutor};
        use uuid::Uuid;

        // Parse workflow_id to extract package and workflow name
        // Support both "package_name/workflow_name" and direct "workflow_name" (defaults to distri_local package)
        let (package_name, workflow_name) = {
            let parts: Vec<&str> = workflow_id.split('/').collect();
            if parts.len() == 1 {
                // Direct workflow name - try distri_local package first
                ("distri_local", parts[0])
            } else if parts.len() == 2 {
                // Standard package_name/workflow_name format
                (parts[0], parts[1])
            } else {
                return Err(anyhow!("Invalid workflow ID format. Expected 'workflow_name' or 'package_name/workflow_name', got: {}", workflow_id));
            }
        };

        // Create execution context for the plugin
        let plugin_context = PluginContext {
            call_id: format!("workflow-{}", Uuid::new_v4()),
            agent_id: Some(context.agent_id.clone()),
            session_id: Some(context.session_id.clone()),
            task_id: Some(context.task_id.clone()),
            run_id: Some(context.run_id.clone()),
            user_id: Some(context.user_id.clone()),
            params: params.clone(),
            secrets: std::collections::HashMap::new(), // TODO: Load secrets if needed for workflows
            env_vars: context.env_vars.clone(),
        };

        // Create workflow execution parameters

        let start_time = std::time::Instant::now();

        // Set workflow runtime for TypeScript executors
        let registry = dap_registry.read().await;

        let result = registry
            .plugin_system
            .execute_workflow(package_name, workflow_name, params, plugin_context)
            .await
            .map_err(|e| anyhow!("Workflow execution failed: {}", e));

        let execution_time_ms = start_time.elapsed().as_millis() as u64;

        let (success, result, error) = match result {
            Ok(result) => (true, Some(result), None),
            Err(e) => (false, None, Some(e.to_string())),
        };
        // Convert plugin result to WorkflowResult
        Ok(WorkflowResult {
            event_id: format!("event-{}", Uuid::new_v4()),
            workflow_id: workflow_id.to_string(),
            success,
            output_data: result,
            error,
            execution_time_ms,
            agent_calls: vec![],
            logs: vec![],
            created_at: Utc::now(),
        })
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
