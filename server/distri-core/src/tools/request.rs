//! HTTP request execution with variable resolution.
//!
//! Provides `execute_http_request` which resolves `$VAR_NAME` references,
//! handles `x-connection-id` OAuth injection, and executes the request.
//!
//! Used by the `POST /request` server route.

use std::collections::HashMap;
use std::sync::Arc;

use distri_types::http_request::{HttpRequestInput, HttpRequestResponse, HttpMethod};
use distri_types::{Part, Tool, ToolContext};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::resolve::{
    extract_vars, extract_vars_from_value, resolve_all, resolve_connection_token,
    substitute_string, ResolveContext,
};
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;

/// Server-side HTTP request tool.
///
/// Registered as a builtin so the LLM sees it in the tool list. When a CLI
/// client is connected, the client intercepts the call via `ExternalToolRegistry`
/// and executes it client-side (or proxies via `POST /request`). When no client
/// is connected (self-hosted distri-server), the server executes it directly.
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

    fn get_parameters(&self) -> serde_json::Value {
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
        let input: HttpRequestInput = serde_json::from_value(tool_call.input)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid input: {}", e)))?;

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

        let result = execute_http_request(&input, &resolve_ctx)
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        Ok(vec![Part::Data(serde_json::to_value(&result).unwrap_or_default())])
    }
}

/// Useful response headers to include (skip noise like cache-control, x-powered-by).
const USEFUL_HEADERS: &[&str] = &[
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

/// Execute an HTTP request with variable resolution.
///
/// Resolves `$VAR_NAME` references in url, headers, and body from the
/// provided `ResolveContext`. Handles `x-connection-id` for OAuth Bearer
/// token injection.
pub async fn execute_http_request(
    input: &HttpRequestInput,
    resolve_ctx: &ResolveContext,
) -> Result<HttpRequestResponse, anyhow::Error> {
    // 1. Check for x-connection-id (consumed, not forwarded)
    let connection_id = input.headers.get("x-connection-id").cloned();

    // 2. Collect all $VAR references
    let mut all_vars = extract_vars(&input.url);
    for (k, v) in &input.headers {
        all_vars.extend(extract_vars(k));
        all_vars.extend(extract_vars(v));
    }
    if let Some(ref body) = input.body {
        all_vars.extend(extract_vars_from_value(body));
    }
    all_vars.sort();
    all_vars.dedup();

    // 3. Resolve variables
    let resolved = resolve_all(&all_vars, resolve_ctx)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // 4. Substitute resolved values
    let url = substitute_string(&input.url, &resolved);
    let headers: HashMap<String, String> = input
        .headers
        .iter()
        .filter(|(k, _)| k.as_str() != "x-connection-id")
        .map(|(k, v)| (substitute_string(k, &resolved), substitute_string(v, &resolved)))
        .collect();
    let body = input
        .body
        .as_ref()
        .map(|b| distri_types::resolve::substitute_value(b, &resolved));

    // 5. Build request headers
    let mut header_map = reqwest::header::HeaderMap::new();
    let mut has_content_type = false;

    for (key, value) in &headers {
        if let (Ok(name), Ok(hval)) = (
            reqwest::header::HeaderName::from_bytes(key.to_lowercase().as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            if name == reqwest::header::CONTENT_TYPE {
                has_content_type = true;
            }
            header_map.insert(name, hval);
        }
    }

    // 6. If x-connection-id was present, resolve and inject Bearer token
    if let Some(ref conn_id) = connection_id {
        let (_provider, access_token) = resolve_connection_token(conn_id, resolve_ctx)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        header_map.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", access_token).parse().unwrap(),
        );
    }

    // 7. Build and send request
    let client = reqwest::Client::new();
    let method_str = input.method.to_string();
    let mut request = match input.method {
        HttpMethod::GET => client.get(&url),
        HttpMethod::POST => client.post(&url),
        HttpMethod::PUT => client.put(&url),
        HttpMethod::PATCH => client.patch(&url),
        HttpMethod::DELETE => client.delete(&url),
    };

    request = request.headers(header_map);

    if let Some(ref body) = body {
        if method_str != "GET" && method_str != "DELETE" {
            if !has_content_type {
                request = request.json(body);
            } else {
                let body_str = match body {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                request = request.body(body_str);
            }
        }
    }

    let response = request
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await?;

    // 8. Read response
    let status = response.status().as_u16();
    let response_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .filter(|(k, _)| USEFUL_HEADERS.contains(&k.as_str()))
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let content_type = response_headers
        .get("content-type")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let response_text = response.text().await.unwrap_or_default();

    let body = if content_type.contains("application/json") {
        serde_json::from_str(&response_text)
            .unwrap_or_else(|_| serde_json::Value::String(response_text))
    } else if response_text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(response_text)
    };

    Ok(HttpRequestResponse {
        status,
        ok: (200..300).contains(&status),
        headers: response_headers,
        body,
    })
}
