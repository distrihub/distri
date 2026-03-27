//! Generic `api_request` tool that makes authenticated HTTP requests to the
//! Distri platform API using the existing `Distri` client.
//!
//! Provides:
//! - `ApiRequestTool` — implements `Tool` trait for server-side execution (gateway/channels)
//! - `execute_api_request()` — shared execution logic, also used by CLI's ExternalToolRegistry
//! - `api_request_definition()` — JSON schema sent as metadata by CLI and web UI

use std::sync::Arc;

use async_trait::async_trait;
use distri_types::{Part, Tool, ToolCall, ToolContext};
use serde_json::{Value, json};

use crate::Distri;

/// Server-side `api_request` tool backed by a `Distri` client.
/// Implements the `Tool` trait so it can be registered as a dynamic tool on the orchestrator.
pub struct ApiRequestTool {
    client: Arc<Distri>,
}

impl ApiRequestTool {
    pub fn new(client: Distri) -> Self {
        Self {
            client: Arc::new(client),
        }
    }
}

impl std::fmt::Debug for ApiRequestTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiRequestTool")
            .field("base_url", &self.client.base_url())
            .finish()
    }
}

#[async_trait]
impl Tool for ApiRequestTool {
    fn get_name(&self) -> String {
        "api_request".to_string()
    }

    fn get_description(&self) -> String {
        "Make authenticated HTTP requests to the Distri API. Use the distri_api skill for available endpoints.".to_string()
    }

    fn get_parameters(&self) -> Value {
        api_request_definition()["parameters"].clone()
    }

    async fn execute(
        &self,
        tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let result = execute_api_request(&self.client, &tool_call.input).await;
        Ok(vec![Part::Data(result)])
    }
}

/// Executes an api_request call using the Distri client's HTTP transport.
///
/// Input: `{ method, path, body?, headers? }`
/// Returns: `{ status, data }` or `{ status, error }`
pub async fn execute_api_request(
    client: &Distri,
    input: &Value,
) -> Value {
    let method = input
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("/");
    let body = input.get("body");
    let extra_headers = input.get("headers");

    let url = format!(
        "{}{}",
        client.base_url().trim_end_matches('/'),
        if path.starts_with('/') { path.to_string() } else { format!("/{}", path) }
    );

    let reqwest_method = match method.to_uppercase().as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };

    let mut request = client.http_client().request(reqwest_method, &url);
    request = request.header("Content-Type", "application/json");

    // Extra headers from agent input
    if let Some(headers) = extra_headers {
        if let Some(obj) = headers.as_object() {
            for (k, v) in obj {
                if let Some(val) = v.as_str() {
                    request = request.header(k.as_str(), val);
                }
            }
        }
    }

    // Body
    if let Some(body) = body {
        if method != "GET" {
            request = request.json(body);
        }
    }

    // Execute
    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return json!({ "status": 0, "error": format!("Request failed: {}", e) });
        }
    };

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();

    let payload: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));

    // Unwrap { data: ... } envelope if present (matches client-side behavior)
    let data = if let Some(obj) = payload.as_object() {
        if let Some(inner) = obj.get("data") {
            inner.clone()
        } else {
            payload.clone()
        }
    } else {
        payload
    };

    if status >= 400 {
        let error_msg = if let Some(obj) = data.as_object() {
            obj.get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("Request failed")
                .to_string()
        } else {
            "Request failed".to_string()
        };
        json!({ "status": status, "error": error_msg })
    } else {
        json!({ "status": status, "data": data })
    }
}

/// Returns the JSON schema definition for the api_request tool.
/// Used by both CLI (metadata) and cloud (Tool trait).
pub fn api_request_definition() -> Value {
    json!({
        "name": "api_request",
        "description": "Make authenticated HTTP requests to the Distri API. Use the distri_api skill for available endpoints.",
        "parameters": {
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"]
                },
                "path": {
                    "type": "string",
                    "description": "API path e.g. /v1/agents, /v1/skills/{id}"
                },
                "body": {
                    "type": "object",
                    "description": "Request body for POST/PUT/PATCH"
                },
                "headers": {
                    "type": "object",
                    "description": "Additional headers (auth and workspace are automatic)"
                }
            },
            "required": ["method", "path"]
        }
    })
}
