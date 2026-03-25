//! DistriStepExecutor — executes workflow steps using the Distri client.
//!
//! Handles ApiCall (HTTP), ToolCall (client.call_tool), and passes through
//! Checkpoint steps. Script and AgentRun are deferred (return mock results).

use crate::Distri;
use distri_workflow::*;
use serde_json::{json, Value};
use std::sync::Arc;

/// A workflow step executor that uses the Distri HTTP client
/// to make real API calls and tool invocations.
pub struct DistriStepExecutor {
    client: Arc<Distri>,
}

impl DistriStepExecutor {
    pub fn new(client: Arc<Distri>) -> Self {
        Self { client }
    }

    async fn execute_api_call(
        &self,
        method: &str,
        url: &str,
        body: &Option<Value>,
        headers: &Option<std::collections::HashMap<String, String>>,
        context: &Value,
    ) -> Result<StepResult, String> {
        // Resolve context variables in URL (e.g., {context.api_base})
        let resolved_url = resolve_template(url, context);

        let mut request = match method.to_uppercase().as_str() {
            "GET" => self.client.http.get(&resolved_url),
            "POST" => self.client.http.post(&resolved_url),
            "PUT" => self.client.http.put(&resolved_url),
            "DELETE" => self.client.http.delete(&resolved_url),
            "PATCH" => self.client.http.patch(&resolved_url),
            _ => return Err(format!("Unsupported HTTP method: {}", method)),
        };

        if let Some(hdrs) = headers {
            for (k, v) in hdrs {
                request = request.header(k, v);
            }
        }

        if let Some(b) = body {
            let resolved = resolve_value(b, context);
            request = request.json(&resolved);
        }

        match request.send().await {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                let response_body: Value = resp.json().await.unwrap_or(json!(null));

                if status_code >= 200 && status_code < 300 {
                    Ok(StepResult::done_with_context(
                        json!({
                            "status": status_code,
                            "body": response_body,
                        }),
                        json!({
                            "last_response": response_body,
                        }),
                    ))
                } else {
                    Ok(StepResult::failed(&format!(
                        "HTTP {} — {}",
                        status_code,
                        serde_json::to_string(&response_body).unwrap_or_default()
                    )))
                }
            }
            Err(e) => Ok(StepResult::failed(&format!("Request failed: {}", e))),
        }
    }

    async fn execute_tool_call(
        &self,
        tool_name: &str,
        input: &Value,
        context: &Value,
    ) -> Result<StepResult, String> {
        let resolved_input = resolve_value(input, context);

        let tool_call = distri_types::ToolCall {
            tool_call_id: uuid::Uuid::new_v4().to_string(),
            tool_name: tool_name.to_string(),
            input: resolved_input,
        };

        match self.client.call_tool(&tool_call, None, None).await {
            Ok(response) => {
                let result_value = json!({
                    "tool_name": tool_name,
                    "response": response,
                });
                Ok(StepResult::done(result_value))
            }
            Err(e) => Ok(StepResult::failed(&format!(
                "Tool call '{}' failed: {}",
                tool_name, e
            ))),
        }
    }
}

#[async_trait::async_trait]
impl StepExecutor for DistriStepExecutor {
    async fn execute(
        &self,
        step: &WorkflowStep,
        context: &Value,
    ) -> Result<StepResult, String> {
        match &step.kind {
            StepKind::ApiCall {
                method,
                url,
                body,
                headers,
            } => self.execute_api_call(method, url, body, headers, context).await,

            StepKind::ToolCall {
                tool_name, input, ..
            } => self.execute_tool_call(tool_name, input, context).await,

            StepKind::Checkpoint { message } => {
                tracing::info!("Checkpoint: {}", message);
                Ok(StepResult::done(json!({ "message": message })))
            }

            StepKind::Script { command, .. } => {
                // Deferred — return info about what would execute
                Ok(StepResult::done(json!({
                    "deferred": true,
                    "command": command,
                    "message": "Script execution not yet implemented in client executor"
                })))
            }

            StepKind::AgentRun {
                agent_id, prompt, ..
            } => {
                // Deferred — return info about what would execute
                Ok(StepResult::done(json!({
                    "deferred": true,
                    "agent_id": agent_id,
                    "prompt": prompt,
                    "message": "Agent execution not yet implemented in client executor"
                })))
            }

            StepKind::Condition { expression, .. } => {
                // Simple condition evaluation — for now just log it
                Ok(StepResult::done(json!({
                    "expression": expression,
                    "evaluated": true,
                    "message": "Condition evaluation is placeholder"
                })))
            }
        }
    }

    fn supports(&self, requirement: &StepRequirement) -> bool {
        // The Distri client supports network calls and tool calls
        match requirement.skill.as_str() {
            "native:network" => true,
            "native:tool" => true,
            _ => false,
        }
    }

    fn available_skills(&self) -> Vec<StepRequirement> {
        vec![
            StepRequirement::native("network"),
            StepRequirement::native("tool"),
        ]
    }
}

/// Resolve `{context.key}` placeholders in a string.
fn resolve_template(template: &str, context: &Value) -> String {
    let mut result = template.to_string();
    if let Some(obj) = context.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{context.{}}}", key);
            if let Some(s) = value.as_str() {
                result = result.replace(&placeholder, s);
            } else {
                result = result.replace(&placeholder, &value.to_string());
            }
        }
    }
    result
}

/// Resolve context references in JSON values.
fn resolve_value(value: &Value, context: &Value) -> Value {
    match value {
        Value::String(s) => {
            if s.starts_with("{context.") && s.ends_with('}') {
                let key = &s[9..s.len() - 1];
                if let Some(v) = context.get(key) {
                    return v.clone();
                }
            }
            Value::String(resolve_template(s, context))
        }
        Value::Object(map) => {
            let resolved: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), resolve_value(v, context)))
                .collect();
            Value::Object(resolved)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| resolve_value(v, context)).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_template() {
        let ctx = json!({
            "api_base": "http://localhost:8086/v1",
            "file_id": "abc123"
        });

        assert_eq!(
            resolve_template("{context.api_base}/files/{context.file_id}", &ctx),
            "http://localhost:8086/v1/files/abc123"
        );
    }

    #[test]
    fn test_resolve_value_string() {
        let ctx = json!({ "class_id": "xyz" });
        let v = json!("{context.class_id}");
        assert_eq!(resolve_value(&v, &ctx), json!("xyz"));
    }

    #[test]
    fn test_resolve_value_nested_object() {
        let ctx = json!({ "title": "My Activity" });
        let v = json!({ "name": "{context.title}", "count": 5 });
        let resolved = resolve_value(&v, &ctx);
        assert_eq!(resolved["name"], "My Activity");
        assert_eq!(resolved["count"], 5);
    }
}
