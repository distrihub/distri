use std::sync::Arc;

use anyhow::Result;
use distri_types::dynamic_tool::DynamicToolFactory;
use distri_types::http_request::{HttpFactoryConfig, HttpFactoryToolInput};
use distri_types::{Part, Tool, ToolContext};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::request::execute_http_request;
use crate::tools::resolve::ResolveContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;

/// Create a dynamic tool from a factory definition.
pub fn create_dynamic_tool(factory: &DynamicToolFactory) -> Result<Arc<dyn ExecutorContextTool>> {
    match factory.factory_type.as_str() {
        "http" => {
            let config: HttpFactoryConfig =
                serde_json::from_value(factory.config.clone()).map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid http factory config for '{}': {}",
                        factory.name,
                        e
                    )
                })?;
            Ok(Arc::new(HttpFactoryTool {
                name: factory.name.clone(),
                config,
                description: factory.description.clone(),
            }))
        }
        "client" => {
            // Client-handled tool — server registers the schema so the LLM can call it,
            // but execution is delegated back to the client via external tool handling.
            let parameters = factory
                .config
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            Ok(Arc::new(ClientDelegatedTool {
                name: factory.name.clone(),
                description: factory
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("Client-handled tool: {}", factory.name)),
                parameters,
            }))
        }
        other => anyhow::bail!(
            "Unknown dynamic tool factory type '{}' for tool '{}'",
            other,
            factory.name
        ),
    }
}

/// Validate a factory definition (used at agent push time).
pub fn validate_dynamic_tool(factory: &DynamicToolFactory) -> Result<()> {
    match factory.factory_type.as_str() {
        "http" => {
            let config: HttpFactoryConfig =
                serde_json::from_value(factory.config.clone()).map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid http factory config for '{}': {}",
                        factory.name,
                        e
                    )
                })?;
            if config.base_url.is_empty() {
                anyhow::bail!(
                    "Dynamic tool '{}': base_url cannot be empty",
                    factory.name
                );
            }
            Ok(())
        }
        "client" => Ok(()), // Client-handled tools are always valid
        other => anyhow::bail!(
            "Unknown dynamic tool factory type '{}' for tool '{}'",
            other,
            factory.name
        ),
    }
}

/// Client-delegated tool — the server registers the tool schema so the LLM includes
/// it in its tool list, but actual execution is handled by the client.
#[derive(Debug)]
struct ClientDelegatedTool {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[async_trait::async_trait]
impl Tool for ClientDelegatedTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn is_external(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "Tool '{}' is client-delegated",
            self.name
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ClientDelegatedTool {
    async fn execute_with_executor_context(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        Err(AgentError::ToolExecution(format!(
            "Tool '{}' is client-delegated",
            self.name
        )))
    }
}

/// HTTP factory tool — created from a DynamicToolFactory with type = "http".
#[derive(Debug)]
struct HttpFactoryTool {
    name: String,
    config: HttpFactoryConfig,
    description: Option<String>,
}

#[async_trait::async_trait]
impl Tool for HttpFactoryTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| format!("Make HTTP requests to {} API", self.name))
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Request path (appended to base URL). Use for platform API calls."
                },
                "url": {
                    "type": "string",
                    "description": "Absolute URL for external API calls (e.g. https://googleapis.com/...). When set, base URL is NOT prepended. Set x-connection-id header to auto-inject OAuth token."
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP method (default: GET)"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Request headers. Set 'x-connection-id' to a connection ID to auto-inject OAuth Bearer token for external API calls."
                },
                "body": {
                    "description": "Request body (sent as JSON by default)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("{} requires ExecutorContext", self.name))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for HttpFactoryTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input: HttpFactoryToolInput = serde_json::from_value(tool_call.input).map_err(|e| {
            AgentError::ToolExecution(format!("{}: invalid input: {}", self.name, e))
        })?;

        // Build full HttpRequestInput from factory config + per-call input
        let request = self.config.build_request(&input);

        // Build ResolveContext from ExecutorContext
        let env_vars = context.env_vars.read().await.clone();
        // Get stores from orchestrator (canonical source for connection stores, secrets, etc.)
        let orch_stores = context.orchestrator.as_ref().map(|o| &o.stores);
        let secret_store = orch_stores.and_then(|s| s.secret_store.clone());

        let resolve_ctx = ResolveContext {
            env_vars,
            secret_store,
        };

        let result = execute_http_request(&request, &resolve_ctx, orch_stores)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("{}: {}", self.name, e)))?;

        Ok(vec![Part::Data(
            serde_json::to_value(&result).unwrap_or_default(),
        )])
    }
}
