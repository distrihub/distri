//! Generic `api_request` tool that makes authenticated HTTP requests.
//!
//! Supports two modes:
//! 1. **Platform API** — `path` param → calls Distri API (e.g., `/agents`, `/skills`)
//! 2. **Connection proxy** — `url` param + `connection_id` → proxies to external APIs
//!    (Google, Slack, Notion, etc.) with OAuth token auto-injected
//!
//! Provides:
//! - `ApiRequestTool` — implements `Tool` trait for server-side execution
//! - `execute_api_request()` — shared execution logic, also used by CLI
//! - `api_request_definition()` — JSON schema sent as metadata

use std::sync::Arc;

use async_trait::async_trait;
use distri_types::{Part, Tool, ToolCall, ToolContext};
use serde_json::{Value, json};

use crate::Distri;

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
        "Make HTTP requests. For Distri API: use `path`. For external APIs (Google, Slack, etc.): use `url` + `connection_id` to auto-inject OAuth token.".to_string()
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

/// Executes an api_request call.
///
/// Two modes:
/// - `path` set → Platform API call (e.g., GET /agents)
/// - `url` set + `connection_id` → Proxy to external API with OAuth token
pub async fn execute_api_request(
    client: &Distri,
    input: &Value,
) -> Value {
    let method = input
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    let path = input.get("path").and_then(|v| v.as_str());
    let external_url = input.get("url").and_then(|v| v.as_str());
    let connection_id = input.get("connection_id").and_then(|v| v.as_str());
    let body = input.get("body");
    let extra_headers = input.get("headers");

    // Determine if this is a connection proxy request or a platform API call
    if let Some(url) = external_url {
        // External API call via connection proxy
        if let Some(conn_id) = connection_id {
            return execute_connection_proxy(client, conn_id, method, url, body, extra_headers).await;
        } else {
            return json!({
                "error": "External URL requests require a `connection_id` for authentication. Use `path` for Distri platform API calls."
            });
        }
    }

    // Platform API call
    let path = path.unwrap_or("/");
    let url = format!(
        "{}{}",
        client.base_url().trim_end_matches('/'),
        if path.starts_with('/') { path.to_string() } else { format!("/{}", path) }
    );

    let reqwest_method = parse_method(method);

    let mut request = client.http_client().request(reqwest_method, &url);
    request = request.header("Content-Type", "application/json");

    if let Some(headers) = extra_headers
        && let Some(obj) = headers.as_object() {
            for (k, v) in obj {
                if let Some(val) = v.as_str() {
                    request = request.header(k.as_str(), val);
                }
            }
        }

    if let Some(body) = body
        && method != "GET" {
            request = request.json(body);
        }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return json!({ "status": 0, "error": format!("Request failed: {}", e) });
        }
    };

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let payload: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));

    // Unwrap { data: ... } envelope if present
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

/// Proxy an external API call through the connection endpoint.
/// Calls POST /connections/{id}/request on the platform, which handles token injection.
async fn execute_connection_proxy(
    client: &Distri,
    connection_id: &str,
    method: &str,
    url: &str,
    body: Option<&Value>,
    extra_headers: Option<&Value>,
) -> Value {
    let proxy_path = format!("/connections/{}/request", connection_id);
    let proxy_url = format!(
        "{}{}",
        client.base_url().trim_end_matches('/'),
        proxy_path
    );

    let mut proxy_body = json!({
        "method": method,
        "url": url,
    });

    if let Some(headers) = extra_headers {
        proxy_body["headers"] = headers.clone();
    }
    if let Some(body) = body {
        proxy_body["body"] = body.clone();
    }

    let response = match client.http_client()
        .post(&proxy_url)
        .header("Content-Type", "application/json")
        .json(&proxy_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return json!({ "error": format!("Connection proxy failed: {}", e) });
        }
    };

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let payload: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));

    if status >= 400 {
        json!({ "error": format!("Connection request failed ({})", status), "details": payload })
    } else {
        // The proxy returns {status, headers, body} — extract the body
        if let Some(obj) = payload.as_object() {
            if let Some(body) = obj.get("body") {
                json!({ "status": status, "data": body })
            } else {
                json!({ "status": status, "data": payload })
            }
        } else {
            json!({ "status": status, "data": payload })
        }
    }
}

fn parse_method(method: &str) -> reqwest::Method {
    match method.to_uppercase().as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    }
}

/// JSON schema definition for the api_request tool.
pub fn api_request_definition() -> Value {
    json!({
        "name": "api_request",
        "description": "Make HTTP requests. For Distri platform API: use `path`. For external APIs (Google, Slack, Notion, etc.): use `url` + `connection_id` to auto-inject OAuth token.",
        "parameters": {
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"],
                    "description": "HTTP method"
                },
                "path": {
                    "type": "string",
                    "description": "Distri API path (e.g., /agents, /skills). Mutually exclusive with `url`."
                },
                "url": {
                    "type": "string",
                    "description": "External API URL (e.g., https://www.googleapis.com/...). Requires `connection_id`."
                },
                "connection_id": {
                    "type": "string",
                    "description": "Connection ID for OAuth token injection. Required when using `url`."
                },
                "body": {
                    "type": "object",
                    "description": "Request body for POST/PUT/PATCH"
                },
                "headers": {
                    "type": "object",
                    "description": "Additional headers"
                }
            },
            "required": ["method"]
        }
    })
}
