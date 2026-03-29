//! HTTP request tool — allows agents to call external APIs.
//!
//! Supports `$VAR_NAME` resolution in url, headers, and body from:
//! - Environment variables (highest priority)
//! - Secret store
//!
//! Supports `x-connection-id` header for OAuth token injection:
//! when present, the tool fetches an OAuth token via the configured
//! token fetcher and injects it as `Authorization: Bearer <token>`.

use std::sync::Arc;

use crate::tools::resolve::{
    extract_vars, extract_vars_from_value, resolve_all, resolve_connection_token,
    substitute_string, substitute_value, ResolveContext,
};
use crate::{agent::ExecutorContext, tools::ExecutorContextTool, types::ToolCall, AgentError};
use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct RequestTool;

#[async_trait::async_trait]
impl Tool for RequestTool {
    fn get_name(&self) -> String {
        "request".to_string()
    }

    fn get_description(&self) -> String {
        "Make an HTTP request to an API. Use $VAR_NAME in url, headers, or body to \
         reference environment variables or secrets (resolved automatically). \
         Add an x-connection-id header to inject an OAuth Bearer token for that connection."
            .to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "RequestInput",
            "type": "object",
            "required": ["url", "method"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Absolute URL. Use $VAR_NAME for variable substitution."
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP method"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Request headers. Use $VAR_NAME for variable substitution. \
                                    Set x-connection-id to inject an OAuth Bearer token."
                },
                "body": {
                    "description": "Request body (sent as JSON). Use $VAR_NAME for variable substitution."
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
        Err(anyhow::anyhow!("RequestTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for RequestTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input = &tool_call.input;

        // 1. Parse input
        let method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let raw_url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'url' parameter".into()))?
            .to_string();

        let headers_value = input.get("headers").cloned().unwrap_or_else(|| json!({}));
        let body_value = input.get("body").cloned();

        // 2. Check for x-connection-id
        let connection_id = headers_value
            .as_object()
            .and_then(|h| h.get("x-connection-id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // 3. Build ResolveContext
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

        // 4. Collect all $VAR references
        let mut all_vars = extract_vars(&raw_url);
        all_vars.extend(extract_vars_from_value(&headers_value));
        if let Some(ref body) = body_value {
            all_vars.extend(extract_vars_from_value(body));
        }
        all_vars.sort();
        all_vars.dedup();

        // 5. Resolve variables
        let resolved = resolve_all(&all_vars, &resolve_ctx)
            .await
            .map_err(|e| AgentError::ToolExecution(e))?;

        // 6. Substitute resolved values
        let url = substitute_string(&raw_url, &resolved);
        let headers_value = substitute_value(&headers_value, &resolved);
        let body_value = body_value.map(|b| substitute_value(&b, &resolved));

        // 7. Build HeaderMap
        let mut header_map = reqwest::header::HeaderMap::new();
        header_map.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );

        if let Some(headers_obj) = headers_value.as_object() {
            for (key, value) in headers_obj {
                // Skip x-connection-id — it's not forwarded as a header
                if key == "x-connection-id" {
                    continue;
                }
                if let Some(val) = value.as_str() {
                    if let (Ok(name), Ok(hval)) = (
                        reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                        reqwest::header::HeaderValue::from_str(val),
                    ) {
                        header_map.insert(name, hval);
                    }
                }
            }
        }

        // If x-connection-id was present, resolve and inject Bearer token
        if let Some(ref conn_id) = connection_id {
            let (_provider, access_token) = resolve_connection_token(conn_id, &resolve_ctx)
                .await
                .map_err(|e| AgentError::ToolExecution(e))?;
            header_map.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", access_token).parse().unwrap(),
            );
        }

        // 8. Build and send request
        let client = reqwest::Client::new();
        let mut request = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            "DELETE" => client.delete(&url),
            _ => {
                return Err(AgentError::ToolExecution(format!(
                    "Unsupported method: {}",
                    method
                )))
            }
        };

        request = request.headers(header_map);

        if let Some(body) = &body_value {
            if method != "GET" && method != "DELETE" {
                request = request.json(body);
            }
        }

        let response = request
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("HTTP request failed: {e}")))?;

        // 9. Read response
        let status = response.status().as_u16();
        let response_text = response.text().await.unwrap_or_default();
        let response_body: Value = serde_json::from_str(&response_text).unwrap_or_else(|_| {
            if response_text.is_empty() {
                json!(null)
            } else {
                json!(response_text)
            }
        });

        // 10. Return consistent format
        let result = if (200..300).contains(&status) {
            let data = response_body
                .get("data")
                .cloned()
                .unwrap_or(response_body.clone());
            json!({
                "status": status,
                "ok": true,
                "data": data,
            })
        } else {
            let error = response_body
                .get("error")
                .cloned()
                .unwrap_or(response_body.clone());
            json!({
                "status": status,
                "ok": false,
                "error": error,
            })
        };

        Ok(vec![Part::Data(result)])
    }
}
