//! HTTP request tool — allows agents to call external APIs.
//!
//! Best for short text and JSON responses (API calls, webhooks, REST endpoints).
//! For large responses, binary data, or streaming — use a browsr shell session instead.
//!
//! Supports `$VAR_NAME` resolution in url, headers, and body from:
//! - Environment variables (highest priority)
//! - Secret store
//!
//! Supports `x-connection-id` header for OAuth token injection:
//! when present, the tool fetches an OAuth token via the configured
//! token fetcher and injects it as `Authorization: Bearer <token>`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::resolve::{
    extract_vars, extract_vars_from_value, resolve_all, resolve_connection_token,
    substitute_string, substitute_value, ResolveContext,
};
use crate::{agent::ExecutorContext, tools::ExecutorContextTool, types::ToolCall, AgentError};
use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct HttpRequestTool;

#[async_trait::async_trait]
impl Tool for HttpRequestTool {
    fn get_name(&self) -> String {
        "http_request".to_string()
    }

    fn get_description(&self) -> String {
        "Make an HTTP request. Best for short text/JSON API responses. \
         For large responses, binary data, or streaming use a browsr shell session instead. \
         Use $VAR_NAME in url, headers, or body to reference environment variables or secrets. \
         Add an x-connection-id header to inject an OAuth Bearer token for that connection."
            .to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "HttpRequestInput",
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
                    "description": "Request body. Sent as JSON if no Content-Type header is set, \
                                    otherwise sent as-is. Use $VAR_NAME for variable substitution."
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
        Err(anyhow::anyhow!("HttpRequestTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for HttpRequestTool {
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

        // 7. Build request headers — only add what the caller specified
        let mut header_map = reqwest::header::HeaderMap::new();
        let mut has_content_type = false;

        if let Some(headers_obj) = headers_value.as_object() {
            for (key, value) in headers_obj {
                if key == "x-connection-id" {
                    continue; // consumed, not forwarded
                }
                if let Some(val) = value.as_str() {
                    if let (Ok(name), Ok(hval)) = (
                        reqwest::header::HeaderName::from_bytes(key.to_lowercase().as_bytes()),
                        reqwest::header::HeaderValue::from_str(val),
                    ) {
                        if name == reqwest::header::CONTENT_TYPE {
                            has_content_type = true;
                        }
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
                if !has_content_type {
                    // Default to JSON when no Content-Type is explicitly set
                    request = request.json(body);
                } else {
                    // Caller set their own Content-Type — send body as-is
                    let body_str = match body {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    request = request.body(body_str);
                }
            }
        }

        let response = request
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("HTTP request failed: {e}")))?;

        // 9. Read response — respect Content-Type, don't assume JSON
        let status = response.status().as_u16();

        // Capture only useful response headers — skip noise like cache-control, x-powered-by etc.
        let useful_headers: &[&str] = &[
            "content-type",
            "content-length",
            "location",
            "retry-after",
            "x-request-id",
            "x-ratelimit-limit",
            "x-ratelimit-remaining",
            "x-ratelimit-reset",
            "www-authenticate",
            "link",
        ];
        let response_headers: HashMap<String, String> = response
            .headers()
            .iter()
            .filter(|(k, _)| useful_headers.contains(&k.as_str()))
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let content_type = response_headers
            .get("content-type")
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        let response_text = response.text().await.unwrap_or_default();

        // Only parse as JSON if content-type says so
        let is_json = content_type.contains("application/json");
        let response_body: Value = if is_json {
            serde_json::from_str(&response_text).unwrap_or_else(|_| Value::String(response_text))
        } else if response_text.is_empty() {
            Value::Null
        } else {
            Value::String(response_text)
        };

        // 10. Build result — return body as-is, don't assume any envelope format
        let headers_json: Value = serde_json::to_value(&response_headers).unwrap_or(json!({}));

        let result = json!({
            "status": status,
            "ok": (200..300).contains(&status),
            "headers": headers_json,
            "body": response_body,
        });

        Ok(vec![Part::Data(result)])
    }
}
