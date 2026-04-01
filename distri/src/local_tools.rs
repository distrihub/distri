use std::{collections::HashMap, path::{Path, PathBuf}, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use distri_filesystem::{create_artifact_tools, create_core_filesystem_tools, create_file_system};
use distri_types::{
    AgentEvent, Part, Tool, ToolCall, ToolContext, ToolDefinition, ToolResponse,
    configuration::ObjectStorageConfig,
    stores::{SessionStore, SessionSummary},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> anyhow::Result<Vec<SessionSummary>> {
        let guard = self.data.read().await;

        let mut sessions: Vec<SessionSummary> = guard
            .iter()
            .filter(|(k, _)| namespace.is_none_or(|n| *k == n))
            .map(|(session_id, values)| SessionSummary {
                session_id: session_id.clone(),
                keys: values.keys().cloned().collect(),
                key_count: values.len(),
                updated_at: None,
            })
            .collect();

        sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));

        let offset = offset.unwrap_or(0);
        let limit = limit.unwrap_or(50);

        if offset >= sessions.len() {
            return Ok(Vec::new());
        }

        let end = (offset + limit).min(sessions.len());
        Ok(sessions[offset..end].to_vec())
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

// ---------------------------------------------------------------------------
// ExecuteCommandTool — runs shell commands locally in the workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ExecuteCommandTool {
    workspace_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExecuteCommandParams {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

#[async_trait]
impl Tool for ExecuteCommandTool {
    fn get_name(&self) -> String {
        "execute_command".to_string()
    }

    fn get_description(&self) -> String {
        "Execute a shell command in the local workspace".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory relative to workspace root",
                    "default": "."
                },
                "env": {
                    "type": "object",
                    "description": "Optional environment variables to set",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let params: ExecuteCommandParams = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| anyhow::anyhow!("invalid execute_command parameters: {}", e))?;

        let mut working_dir = self.workspace_root.clone();
        if let Some(ref relative) = params.cwd {
            let trimmed = relative.trim();
            if !trimmed.is_empty() && trimmed != "." {
                working_dir = working_dir.join(trimmed);
            }
        }
        std::fs::create_dir_all(&working_dir)
            .with_context(|| format!("failed to create working directory {:?}", working_dir))?;

        let mut command = if cfg!(target_os = "windows") {
            let mut cmd = tokio::process::Command::new("cmd");
            cmd.arg("/C").arg(&params.command);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-lc").arg(&params.command);
            cmd
        };

        command.current_dir(&working_dir);

        if let Some(env_map) = params.env {
            for (key, value) in env_map {
                command.env(key, value);
            }
        }

        let output = command
            .output()
            .await
            .with_context(|| format!("failed to execute command '{}'", params.command))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or_default();

        let response = json!({
            "command": params.command,
            "cwd": params.cwd.unwrap_or_else(|| ".".to_string()),
            "exit_code": exit_code,
            "success": output.status.success(),
            "stdout": stdout,
            "stderr": stderr
        });

        Ok(vec![Part::Data(response)])
    }
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

    // Register execute_command for local shell execution
    let exec_tool = Arc::new(ExecuteCommandTool {
        workspace_root: workspace_root.to_path_buf(),
    });
    let exec_def = exec_tool.get_tool_definition();
    definitions.push(exec_def);

    let session_store_exec = session_store.clone();
    registry.register(
        agent_id.to_string(),
        "execute_command".to_string(),
        move |call: ToolCall, event: AgentEvent| {
            let tool = exec_tool.clone();
            let session_store = session_store_exec.clone();
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

    Ok(definitions)
}
