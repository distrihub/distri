use crate::agent::ExecutorContext;
use crate::tools::resolve::{resolve_connection_token, ResolveContext};
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Callback type for fetching a connection token given a connection_id.
/// Returns (provider_name, access_token) or an error.
pub type TokenFetcher = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send>>
        + Send
        + Sync,
>;

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

        // Build ResolveContext from ExecutorContext (same pattern as RequestTool)
        let env_vars = context.env_vars.read().await.clone();
        let secret_store = context
            .stores
            .as_ref()
            .and_then(|s| s.secret_store.clone());
        let token_fetcher = context.token_fetcher.clone();

        let resolve_ctx = ResolveContext {
            env_vars,
            secret_store,
            token_fetcher,
        };

        // Fetch token via shared resolution
        let (provider, access_token) = resolve_connection_token(connection_id, &resolve_ctx)
            .await
            .map_err(|e| AgentError::ToolExecution(e))?;

        // Determine env var name
        let env_var_name = input
            .get("env_var")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}_TOKEN", provider.to_uppercase()));

        // Inject into context env_vars (shared via Arc<RwLock>)
        {
            let mut env_vars = context.env_vars.write().await;
            env_vars.insert(env_var_name.clone(), access_token);
        }

        tracing::info!(
            "[inject_connection_env] Injected {} for provider '{}' (connection: {})",
            env_var_name,
            provider,
            connection_id
        );

        // Return confirmation — token never appears in the response
        Ok(vec![Part::Data(json!({
            "injected": true,
            "provider": provider,
            "env_var": env_var_name,
            "connection_id": connection_id,
        }))])
    }
}
