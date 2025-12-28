use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use distri_filesystem::{create_artifact_tools, create_core_filesystem_tools, create_file_system};
use distri_types::{
    AgentEvent, ToolCall, ToolContext, ToolDefinition, ToolResponse,
    configuration::ObjectStorageConfig, stores::{SessionStore, SessionSummary},
};
use tokio::sync::RwLock;

use crate::ExternalToolRegistry;

/// Simple in-memory session store used by local filesystem tool handlers.
#[derive(Debug, Default)]
struct LocalSessionStore {
    data: RwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[async_trait]
impl SessionStore for LocalSessionStore {
    async fn clear_session(&self, namespace: &str) -> anyhow::Result<()> {
        self.data.write().await.remove(namespace);
        Ok(())
    }

    async fn set_value(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut guard = self.data.write().await;
        let entry = guard.entry(namespace.to_string()).or_default();
        entry.insert(key.to_string(), value.clone());
        Ok(())
    }

    async fn set_value_with_expiry(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
        _expiry: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<()> {
        self.set_value(namespace, key, value).await
    }

    async fn get_value(
        &self,
        namespace: &str,
        key: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let guard = self.data.read().await;
        Ok(guard.get(namespace).and_then(|m| m.get(key).cloned()))
    }

    async fn delete_value(&self, namespace: &str, key: &str) -> anyhow::Result<()> {
        let mut guard = self.data.write().await;
        if let Some(entry) = guard.get_mut(namespace) {
            entry.remove(key);
        }
        Ok(())
    }

    async fn get_all_values(
        &self,
        namespace: &str,
    ) -> anyhow::Result<HashMap<String, serde_json::Value>> {
        let guard = self.data.read().await;
        Ok(guard.get(namespace).cloned().unwrap_or_default())
    }

    async fn list_sessions(
        &self,
        namespace: Option<&str>,
    ) -> anyhow::Result<Vec<SessionSummary>> {
        let guard = self.data.read().await;
        let iter: Box<dyn Iterator<Item = (&String, &HashMap<String, serde_json::Value>)>> =
            if let Some(namespace) = namespace {
                guard
                    .get_key_value(namespace)
                    .map(|(key, value)| Box::new(std::iter::once((key, value))))
                    .unwrap_or_else(|| Box::new(std::iter::empty()))
            } else {
                Box::new(guard.iter())
            };

        let sessions = iter
            .map(|(session_id, values)| SessionSummary {
                session_id: session_id.clone(),
                keys: values.keys().cloned().collect(),
                key_count: values.len(),
                updated_at: None,
            })
            .collect();

        Ok(sessions)
    }
}

fn make_tool_context(event: &AgentEvent, session_store: Arc<dyn SessionStore>) -> Arc<ToolContext> {
    Arc::new(ToolContext {
        agent_id: event.agent_id.clone(),
        session_id: event.run_id.clone(),
        task_id: event.task_id.clone(),
        run_id: event.run_id.clone(),
        thread_id: event.thread_id.clone(),
        user_id: "local_user".to_string(),
        session_store,
        event_tx: None,
        metadata: None,
    })
}

/// Register local filesystem and artifact tools for an agent, returning their definitions for UI/listing.
pub async fn register_local_filesystem_tools(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    workspace_root: &Path,
) -> Result<Vec<ToolDefinition>> {
    let fs_config = distri_filesystem::FileSystemConfig {
        object_store: ObjectStorageConfig::FileSystem {
            base_path: workspace_root.to_string_lossy().to_string(),
        },
        root_prefix: None,
    };

    let workspace_fs = Arc::new(create_file_system(fs_config).await?);
    let session_fs = Arc::new(
        workspace_fs
            .scoped(Some(".distri/session_storage"))
            .context("scoping session filesystem")?,
    );

    let filesystem_tools = create_core_filesystem_tools(workspace_fs.clone());
    let artifact_tools = create_artifact_tools(session_fs.clone());

    let session_store: Arc<dyn SessionStore> = Arc::new(LocalSessionStore::default());
    let mut definitions = Vec::new();

    for tool in filesystem_tools
        .into_iter()
        .chain(artifact_tools.into_iter())
    {
        let definition = tool.get_tool_definition();
        let tool_name = definition.name.clone();
        definitions.push(definition);

        let tool_clone = tool.clone();
        let session_store = session_store.clone();
        registry.register(
            agent_id.to_string(),
            tool_name.clone(),
            move |call: ToolCall, event: AgentEvent| {
                let tool = tool_clone.clone();
                let session_store = session_store.clone();
                async move {
                    let context = make_tool_context(&event, session_store.clone());
                    let parts = tool.execute(call.clone(), context).await?;
                    Ok(ToolResponse::from_parts(
                        call.tool_call_id.clone(),
                        tool.get_name(),
                        parts,
                    ))
                }
            },
        );
    }

    Ok(definitions)
}
