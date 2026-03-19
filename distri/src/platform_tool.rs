use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{Value, json};

use crate::{CreateSkillRequest, Distri, ExternalToolRegistry, NewSecretRequest};
use distri_types::{AgentEvent, ToolCall, ToolResponse};

/// A tool handler that exposes Distri platform APIs as a single `distri_platform` tool,
/// routing actions based on the `action` parameter in the tool call input.
#[derive(Clone)]
pub struct PlatformTool {
    client: Arc<Distri>,
}

impl PlatformTool {
    /// Create a new PlatformTool wrapping the given client.
    pub fn new(client: Distri) -> Self {
        Self {
            client: Arc::new(client),
        }
    }

    /// Create a new PlatformTool from an already-Arc'd client.
    pub fn from_arc(client: Arc<Distri>) -> Self {
        Self { client }
    }

    /// Register this tool as a global handler for `("*", "distri_platform")` in the registry.
    /// Errors from `execute` are caught and returned as `{"error": "..."}` values.
    pub fn register(self, registry: &ExternalToolRegistry) {
        let tool = Arc::new(self);
        registry.register("*", "distri_platform", move |call: ToolCall, _event: AgentEvent| {
            let tool = tool.clone();
            async move {
                let action = call
                    .input
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = call.input.get("params").cloned().unwrap_or(json!({}));

                let result = match tool.execute(&action, params).await {
                    Ok(v) => v,
                    Err(e) => json!({ "error": e.to_string() }),
                };

                Ok(ToolResponse::direct(
                    call.tool_call_id.clone(),
                    call.tool_name.clone(),
                    result,
                ))
            }
        });
    }

    /// Route an action name to the appropriate Distri API call.
    pub async fn execute(&self, action: &str, params: Value) -> Result<Value> {
        match action {
            "list_actions" => Ok(json!([
                "list_actions",
                "list_agents",
                "get_agent",
                "list_skills",
                "get_skill",
                "create_skill",
                "delete_skill",
                "list_providers",
                "connect",
                "list_connections",
                "get_connection_token",
                "list_secrets",
                "get_secret",
                "set_secret",
                "delete_secret",
                "list_threads",
            ])),

            "list_agents" => {
                let agents = self.client.list_agents().await?;
                Ok(serde_json::to_value(agents)?)
            }

            "get_agent" => {
                let id = required_param(&params, "agent_id")?;
                let agent = self.client.fetch_agent(&id).await?;
                Ok(serde_json::to_value(agent)?)
            }

            "list_skills" => {
                let skills = self.client.list_skills().await?;
                Ok(serde_json::to_value(skills)?)
            }

            "get_skill" => {
                let id = required_param(&params, "skill_id")?;
                let skill = self.client.get_skill(&id).await?;
                Ok(serde_json::to_value(skill)?)
            }

            "create_skill" => {
                let name = required_param(&params, "name")?;
                let content = required_param(&params, "content")?;
                let req = CreateSkillRequest {
                    name,
                    description: params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    content,
                    tags: vec![],
                    is_public: false,
                    scripts: vec![],
                };
                let skill = self.client.create_skill(&req).await?;
                Ok(serde_json::to_value(skill)?)
            }

            "delete_skill" => {
                let id = required_param(&params, "skill_id")?;
                self.client.delete_skill(&id).await?;
                Ok(json!({ "deleted": true, "skill_id": id }))
            }

            "list_providers" => {
                let providers = self.client.list_providers().await?;
                Ok(serde_json::to_value(providers)?)
            }

            "connect" => {
                let provider = required_param(&params, "provider")?;
                let scopes: Vec<String> = params
                    .get("scopes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let result = self.client.connect(&provider, &scopes).await?;
                Ok(serde_json::to_value(result)?)
            }

            "list_connections" => {
                let connections = self.client.list_connections().await?;
                Ok(serde_json::to_value(connections)?)
            }

            "get_connection_token" => {
                let id = required_param(&params, "connection_id")?;
                let token = self.client.get_connection_token(&id).await?;
                Ok(serde_json::to_value(token)?)
            }

            "list_secrets" => {
                let secrets = self.client.list_secrets().await?;
                Ok(serde_json::to_value(secrets)?)
            }

            "get_secret" => {
                let key = required_param(&params, "key")?;
                let secret = self.client.get_secret(&key).await?;
                Ok(serde_json::to_value(secret)?)
            }

            "set_secret" => {
                let key = required_param(&params, "key")?;
                let value = required_param(&params, "value")?;
                let req = NewSecretRequest { key, value };
                let entry = self.client.set_secret(&req).await?;
                Ok(serde_json::to_value(entry)?)
            }

            "delete_secret" => {
                let key = required_param(&params, "key")?;
                self.client.delete_secret(&key).await?;
                Ok(json!({ "deleted": true, "key": key }))
            }

            "list_threads" => {
                let threads = self.client.list_threads().await?;
                Ok(serde_json::to_value(threads)?)
            }

            _ => Err(anyhow!("Unknown platform action: {action}")),
        }
    }
}

/// Extract a required string parameter from a JSON params object.
fn required_param(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("Missing required parameter: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DistriConfig;

    fn make_tool() -> PlatformTool {
        let client = Distri::from_config(DistriConfig::new("http://localhost:9999"));
        PlatformTool::new(client)
    }

    #[tokio::test]
    async fn test_list_actions() {
        let tool = make_tool();
        let result = tool.execute("list_actions", json!({})).await.unwrap();
        let actions: Vec<String> = serde_json::from_value(result).unwrap();
        assert!(actions.contains(&"list_actions".to_string()));
        assert!(actions.contains(&"list_agents".to_string()));
        assert!(actions.contains(&"list_skills".to_string()));
        assert!(actions.contains(&"list_connections".to_string()));
        assert!(actions.contains(&"list_secrets".to_string()));
        assert!(actions.contains(&"list_threads".to_string()));
        assert_eq!(actions.len(), 14);
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = make_tool();
        let err = tool
            .execute("totally_unknown_action", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Unknown platform action"),
            "Expected 'Unknown platform action' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_get_agent_missing_param() {
        let tool = make_tool();
        let err = tool.execute("get_agent", json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("agent_id"),
            "Expected 'agent_id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_get_skill_missing_param() {
        let tool = make_tool();
        let err = tool.execute("get_skill", json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("skill_id"),
            "Expected 'skill_id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_create_skill_missing_name() {
        let tool = make_tool();
        let err = tool
            .execute("create_skill", json!({ "content": "some content" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("name"),
            "Expected 'name' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_create_skill_missing_content() {
        let tool = make_tool();
        let err = tool
            .execute("create_skill", json!({ "name": "my-skill" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("content"),
            "Expected 'content' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_delete_skill_missing_param() {
        let tool = make_tool();
        let err = tool.execute("delete_skill", json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("skill_id"),
            "Expected 'skill_id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_get_connection_token_missing_param() {
        let tool = make_tool();
        let err = tool
            .execute("get_connection_token", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("connection_id"),
            "Expected 'connection_id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_get_secret_missing_param() {
        let tool = make_tool();
        let err = tool.execute("get_secret", json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "Expected 'key' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_set_secret_missing_key() {
        let tool = make_tool();
        let err = tool
            .execute("set_secret", json!({ "value": "myvalue" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "Expected 'key' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_set_secret_missing_value() {
        let tool = make_tool();
        let err = tool
            .execute("set_secret", json!({ "key": "MY_KEY" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("value"),
            "Expected 'value' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_delete_secret_missing_param() {
        let tool = make_tool();
        let err = tool.execute("delete_secret", json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "Expected 'key' in error, got: {err}"
        );
    }
}
