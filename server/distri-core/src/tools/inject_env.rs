use crate::agent::ExecutorContext;
use crate::connections::{ConnectionResolver, DefaultResolver, ResolveCtx};
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool that fetches a connection token and injects it as an environment variable.
/// The token never appears in conversation messages — only in env_vars map.
/// Child agents (via new_task/continue_as) inherit the env vars automatically.
#[derive(Debug)]
pub struct InjectConnectionEnvTool;

#[async_trait::async_trait]
impl Tool for InjectConnectionEnvTool {
    fn get_name(&self) -> String {
        "inject_connection_env".to_string()
    }

    fn get_description(&self) -> String {
        "Fetch a connection token and inject it as an environment variable. The token is silently added to the execution context — child agents will receive it automatically. Use this before calling a sub-agent that needs API access.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "connection_id": {
                    "type": "string",
                    "description": "The connection ID to fetch the token for"
                },
                "env_var": {
                    "type": "string",
                    "description": "Optional: override the environment variable name (default: <PROVIDER>_TOKEN, e.g. GOOGLE_TOKEN)"
                }
            },
            "required": ["connection_id"]
        })
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "InjectConnectionEnvTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for InjectConnectionEnvTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input = &tool_call.input;

        let connection_id = input
            .get("connection_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing 'connection_id' parameter".to_string())
            })?;

        // Get stores from orchestrator (canonical source for connection stores)
        let stores = context
            .orchestrator
            .as_ref()
            .map(|o| &o.stores)
            .ok_or_else(|| {
                AgentError::ToolExecution(
                    "orchestrator not available for connection resolution".to_string(),
                )
            })?;

        let env_var_override = input.get("env_var").and_then(|v| v.as_str());

        let mut resolve_ctx = ResolveCtx::new(stores);
        if let Some(ws) = context.workspace_id.as_deref() {
            resolve_ctx = resolve_ctx.with_workspace(ws);
        }
        resolve_ctx = resolve_ctx.with_user(context.user_id.as_str());
        if let Some(ev) = env_var_override {
            resolve_ctx = resolve_ctx.with_env_override(ev);
        }

        let resolved = DefaultResolver
            .resolve(connection_id, &resolve_ctx)
            .await
            .map_err(AgentError::ToolExecution)?;

        // Merge resolved env vars into context (shared via Arc<RwLock>).
        let injected_names: Vec<String> = resolved.env_vars.keys().cloned().collect();
        {
            let mut env_vars = context.env_vars.write().await;
            for (k, v) in &resolved.env_vars {
                env_vars.insert(k.clone(), v.clone());
            }
        }
        context.mark_connection_used(connection_id).await;

        tracing::info!(
            "[inject_connection_env] Injected {:?} for provider '{}' (connection: {})",
            injected_names,
            resolved.provider,
            connection_id
        );

        // Return confirmation — tokens never appear in the response
        Ok(vec![Part::Data(json!({
            "injected": true,
            "provider": resolved.provider,
            "env_vars": injected_names,
            "connection_id": connection_id,
        }))])
    }
}
