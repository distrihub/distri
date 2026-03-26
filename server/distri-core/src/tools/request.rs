//! HTTP request tool — allows agents to call external APIs.
//!
//! Reads auth credentials from ExecutorContext env_vars:
//! - `REQUEST_AUTH_TOKEN` — Bearer token (e.g. zippy API key)
//! - `REQUEST_BASE_URL` — optional base URL prepended to relative paths
//! - `REQUEST_ORG_ID` — optional org ID sent as x-org-id header
//!
//! These are injected by the client (zippy-cli or distrijs) via env_vars
//! in the executor context metadata.

use std::sync::Arc;

use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};

use crate::{
    agent::ExecutorContext,
    tools::ExecutorContextTool,
    types::ToolCall,
    AgentError,
};

#[derive(Debug)]
pub struct RequestTool;

#[async_trait::async_trait]
impl Tool for RequestTool {
    fn get_name(&self) -> String {
        "request".to_string()
    }

    fn get_description(&self) -> String {
        "Make an HTTP request to an API. Auth credentials are injected from context — \
         you don't need to provide Authorization headers manually. \
         Use relative URLs (e.g. /admin/activities) when REQUEST_BASE_URL is configured."
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
                    "description": "URL or path (relative paths use REQUEST_BASE_URL from context)"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP method"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Additional headers (auth headers are auto-injected)"
                },
                "body": {
                    "description": "Request body (sent as JSON for POST/PUT/PATCH)"
                }
            },
            "additionalProperties": false
        })
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(r#"
Get an activity:
{"url": "/admin/activities/act-123", "method": "GET"}

Create an activity:
{"url": "/admin/activities", "method": "POST", "body": {"class_id": "cls-1", "title": "My Activity", "activity_source": "import"}}

Update activity config:
{"url": "/admin/activities/act-123", "method": "PATCH", "body": {"config": {"import_status": "review", "detection": {"questions": [], "submissions": []}}}}
"#.to_string())
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

        let method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let raw_url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'url' parameter".into()))?;

        // Read env vars from context
        let env_vars = context.env_vars.read().await;
        let base_url = env_vars.get("REQUEST_BASE_URL").cloned().unwrap_or_default();
        let auth_token = env_vars.get("REQUEST_AUTH_TOKEN").cloned();
        let org_id = env_vars.get("REQUEST_ORG_ID").cloned();
        drop(env_vars);

        // Resolve URL
        let url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
            raw_url.to_string()
        } else if !base_url.is_empty() {
            format!("{}{}", base_url.trim_end_matches('/'), raw_url)
        } else {
            return Err(AgentError::ToolExecution(format!(
                "Relative URL '{}' used but REQUEST_BASE_URL not set in context",
                raw_url
            )));
        };

        // Build request
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

        // Auto-inject auth
        if let Some(token) = &auth_token {
            request = request.bearer_auth(token);
        }
        if let Some(oid) = &org_id {
            request = request.header("x-org-id", oid);
        }
        request = request.header("Content-Type", "application/json");

        // Add user-provided headers
        if let Some(headers) = input.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers {
                if let Some(val) = value.as_str() {
                    request = request.header(key.as_str(), val);
                }
            }
        }

        // Add body
        if let Some(body) = input.get("body") {
            if method != "GET" && method != "DELETE" {
                request = request.json(body);
            }
        }

        // Execute
        let response = request
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("HTTP request failed: {e}")))?;

        let status = response.status().as_u16();
        let response_body: Value = response
            .json()
            .await
            .unwrap_or_else(|_| json!(null));

        // Return structured result
        let result = if (200..300).contains(&status) {
            // Success — return the data field if it exists (zippy wraps in {ok, data, error})
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
