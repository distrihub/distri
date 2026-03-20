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
                // Accept params either nested under "params" key or flat at root level
                let params = if let Some(p) = call.input.get("params") {
                    p.clone()
                } else {
                    // Fallback: use entire input minus "action" as params
                    let mut flat = call.input.clone();
                    if let Some(obj) = flat.as_object_mut() {
                        obj.remove("action");
                    }
                    flat
                };

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
                "get_connection_usage",
                "connection_request",
                "register_connection_provider",
                "list_connection_providers",
                "discover_skill",
                "import_skill",
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
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("Missing 'provider'. Call like: distri_platform with action='connect', provider='google', additional_scopes=['drive','spreadsheets']"))?;
                let scopes: Vec<String> = params
                    .get("scopes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let additional_scopes: Vec<String> = params
                    .get("additional_scopes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                // Check if connection already exists for this provider
                if let Ok(connections) = self.client.list_connections().await {
                    if let Some(existing) = connections.iter().find(|c| c.name == provider) {
                        let status = existing.status.as_deref().unwrap_or("unknown");
                        if status == "connected" {
                            if additional_scopes.is_empty() {
                                let existing_scopes: Vec<String> = existing
                                    .config
                                    .as_ref()
                                    .and_then(|c| c.get("scopes"))
                                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                                    .unwrap_or_default();
                                let hint = capability_hint(&provider, &existing_scopes);

                                return Ok(json!({
                                    "already_connected": true,
                                    "connection_id": existing.id,
                                    "provider": provider,
                                    "status": status,
                                    "current_scopes": existing_scopes,
                                    "capabilities": hint,
                                    "action_required": format!("Use connection_request to make API calls NOW. Do NOT transfer to another agent or use code execution. Example: distri_platform({{action: 'connection_request', params: {{connection_id: '{}', method: 'POST', url: 'https://sheets.googleapis.com/v4/spreadsheets', body: {{properties: {{title: 'My Sheet'}}}}}}}})", existing.id)
                                }));
                            }
                            // Re-trigger OAuth with expanded scopes: merge existing + additional
                            let existing_scopes: Vec<String> = existing
                                .config
                                .as_ref()
                                .and_then(|c| c.get("scopes"))
                                .and_then(|v| serde_json::from_value(v.clone()).ok())
                                .unwrap_or_default();
                            let mut merged: Vec<String> = existing_scopes;
                            for s in &additional_scopes {
                                if !merged.contains(s) {
                                    merged.push(s.clone());
                                }
                            }
                            // Delete the existing connection and recreate with expanded scopes
                            let _ = self.client.delete_connection(&existing.id).await;
                            let result = self.client.connect(&provider, &merged).await?;
                            return Ok(serde_json::to_value(result)?);
                        }
                        // Pending — delete and re-create to get a fresh auth URL
                        let _ = self.client.delete_connection(&existing.id).await;
                    }
                }

                // Combine scopes and additional_scopes for fresh connections
                let mut all_scopes = scopes;
                for s in additional_scopes {
                    if !all_scopes.contains(&s) {
                        all_scopes.push(s);
                    }
                }

                let result = self.client.connect(&provider, &all_scopes).await?;
                Ok(serde_json::to_value(result)?)
            }

            "list_connections" => {
                // Lightweight: return connections with scopes + short capability hints
                let connections = self.client.list_connections().await?;
                let enriched: Vec<Value> = connections
                    .iter()
                    .map(|c| {
                        let scopes: Vec<String> = c
                            .config
                            .as_ref()
                            .and_then(|cfg| cfg.get("scopes"))
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        let hint = capability_hint(&c.name, &scopes);
                        let needs_scopes = needs_scope_upgrade(&c.name, &scopes);
                        let mut entry = json!({
                            "connection_id": c.id,
                            "provider": c.name,
                            "status": c.status.as_deref().unwrap_or("unknown"),
                            "scopes": scopes,
                            "capabilities": hint,
                        });
                        if let Some(upgrade) = needs_scopes {
                            entry["needs_scope_upgrade"] = json!(upgrade);
                        }
                        entry
                    })
                    .collect();
                Ok(json!({
                    "connections": enriched,
                    "important": "Use 'connection_request' to make API calls directly. Do NOT use transfer_to_agent, call_code, or browser for connected services. Example: distri_platform({action: 'connection_request', params: {connection_id: '<id>', method: 'GET', url: 'https://api.example.com/endpoint'}}). Use 'get_connection_usage' for API docs if needed."
                }))
            }

            "get_connection_usage" => {
                let id = required_param(&params, "connection_id")?;
                let detail = self.client.get_connection_detail(&id).await?;
                let skill_content = detail
                    .get("skill")
                    .and_then(|s| s.get("content"))
                    .and_then(|c| c.as_str());
                let scopes: Vec<String> = detail
                    .get("config")
                    .and_then(|c| c.get("scopes"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let provider = detail
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                match skill_content {
                    Some(content) => Ok(json!({
                        "provider": provider,
                        "current_scopes": scopes,
                        "api_usage": content,
                        "tip": format!("Use 'connection_request' with connection_id '{}' to call these APIs. If you need more scopes, use connect({{provider: \"{}\", additional_scopes: [\"scope_name\"]}}).", id, provider)
                    })),
                    None => Ok(json!({
                        "provider": provider,
                        "current_scopes": scopes,
                        "api_usage": format!("No detailed API docs available for {}. Use connection_request with standard REST API endpoints for this provider.", provider),
                    })),
                }
            }

            "connection_request" => {
                let connection_id = params.get("connection_id").and_then(|v| v.as_str()).map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("Missing 'connection_id'. Call like: distri_platform with action='connection_request', connection_id='<id>', method='POST', url='https://sheets.googleapis.com/v4/spreadsheets'"))?;
                let method = required_param(&params, "method")?;
                let url = required_param(&params, "url")?;
                let headers: Option<std::collections::HashMap<String, String>> = params
                    .get("headers")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let body = params.get("body").cloned();

                let result = self
                    .client
                    .connection_request(&connection_id, &method, &url, headers, body)
                    .await?;
                Ok(serde_json::to_value(result)?)
            }

            "register_connection_provider" => {
                let id = required_param(&params, "id")?;
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
                let auth_url = required_param(&params, "authorization_url")?;
                let token_url = required_param(&params, "token_url")?;
                let client_id = required_param(&params, "client_id")?;
                let client_secret = required_param(&params, "client_secret")?;

                let provider_config = json!({
                    "id": id,
                    "name": name,
                    "authorization_url": auth_url,
                    "token_url": token_url,
                    "scopes_supported": params.get("scopes_supported").cloned().unwrap_or(json!([])),
                    "default_scopes": params.get("default_scopes").cloned().unwrap_or(json!([])),
                    "scope_mappings": params.get("scope_mappings").cloned().unwrap_or(json!({})),
                });

                let result = self.client.register_connection_provider(
                    provider_config,
                    &client_id,
                    &client_secret,
                ).await?;
                Ok(result)
            }

            "list_connection_providers" => {
                let providers = self.client.list_connection_providers().await?;
                Ok(json!({
                    "connection_providers": providers,
                    "tip": "These are custom connection providers. Built-in providers (google, github, slack, etc.) are always available via 'list_providers'."
                }))
            }

            "discover_skill" => {
                let query = required_param(&params, "query")?;
                let results = self.client.discover_skills(&query).await?;
                Ok(results)
            }

            "import_skill" => {
                let url = required_param(&params, "url")?;
                let name = params.get("name").and_then(|v| v.as_str());
                let result = self.client.import_skill(&url, name).await?;
                Ok(result)
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

/// If a provider has only basic scopes, return an upgrade suggestion with the exact action to call.
fn needs_scope_upgrade(provider: &str, scopes: &[String]) -> Option<String> {
    let joined = scopes.join(" ");
    match provider {
        "google" => {
            let has_drive = joined.contains("drive") && !joined.contains("drive.readonly");
            let has_sheets = joined.contains("spreadsheet");
            if !has_drive || !has_sheets {
                Some(format!(
                    "To use Google Sheets/Drive, call: distri_platform({{action: 'connect', params: {{provider: 'google', additional_scopes: ['drive', 'spreadsheets']}}}})"
                ))
            } else {
                None
            }
        }
        "slack" => {
            if !joined.contains("chat") {
                Some("To send Slack messages, call: distri_platform({action: 'connect', params: {provider: 'slack', additional_scopes: ['chat:write']}})".to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Return a short, token-efficient capability summary for a provider given its scopes.
fn capability_hint(provider: &str, scopes: &[String]) -> String {
    match provider {
        "google" => {
            let mut caps = Vec::new();
            let joined = scopes.join(" ");
            if joined.contains("calendar") {
                caps.push("Calendar");
            }
            if joined.contains("gmail") {
                caps.push("Gmail");
            }
            if joined.contains("drive") {
                caps.push("Drive");
            }
            if joined.contains("spreadsheet") || joined.contains("sheets") {
                caps.push("Sheets");
            }
            if caps.is_empty() {
                caps.push("Profile only — use additional_scopes to add APIs");
            }
            format!("Google APIs: {}", caps.join(", "))
        }
        "github" => {
            let joined = scopes.join(" ");
            if joined.contains("repo") {
                "GitHub: repos, issues, PRs".to_string()
            } else {
                "GitHub: user profile — use additional_scopes=[\"repo\"] for repos".to_string()
            }
        }
        "slack" => {
            let mut caps = Vec::new();
            let joined = scopes.join(" ");
            if joined.contains("users") {
                caps.push("users");
            }
            if joined.contains("channels") {
                caps.push("channels");
            }
            if joined.contains("chat") {
                caps.push("messaging");
            }
            if caps.is_empty() {
                "Slack: basic access".to_string()
            } else {
                format!("Slack: {}", caps.join(", "))
            }
        }
        "notion" => "Notion: pages, databases, search".to_string(),
        "microsoft" => "Microsoft Graph: mail, calendar, files".to_string(),
        "twitter" => "Twitter/X: tweets, users, search".to_string(),
        _ => format!("{}: connected", provider),
    }
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
        assert!(actions.contains(&"get_connection_usage".to_string()));
        assert!(actions.contains(&"connection_request".to_string()));
        assert!(actions.contains(&"register_connection_provider".to_string()));
        assert!(actions.contains(&"list_connection_providers".to_string()));
        assert!(actions.contains(&"discover_skill".to_string()));
        assert!(actions.contains(&"import_skill".to_string()));
        assert!(actions.contains(&"list_secrets".to_string()));
        assert!(actions.contains(&"list_threads".to_string()));
        assert_eq!(actions.len(), 21);
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

    #[tokio::test]
    async fn test_get_connection_usage_missing_param() {
        let tool = make_tool();
        let err = tool
            .execute("get_connection_usage", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("connection_id"),
            "Expected 'connection_id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_connection_request_missing_params() {
        let tool = make_tool();
        let err = tool
            .execute("connection_request", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("connection_id"),
            "Expected 'connection_id' in error, got: {err}"
        );

        let err = tool
            .execute(
                "connection_request",
                json!({ "connection_id": "abc" }),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("method"),
            "Expected 'method' in error, got: {err}"
        );

        let err = tool
            .execute(
                "connection_request",
                json!({ "connection_id": "abc", "method": "GET" }),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("url"),
            "Expected 'url' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_register_connection_provider_missing_params() {
        let tool = make_tool();
        let err = tool
            .execute("register_connection_provider", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("id"),
            "Expected 'id' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_discover_skill_missing_param() {
        let tool = make_tool();
        let err = tool
            .execute("discover_skill", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("query"),
            "Expected 'query' in error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_import_skill_missing_param() {
        let tool = make_tool();
        let err = tool
            .execute("import_skill", json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("url"),
            "Expected 'url' in error, got: {err}"
        );
    }
}
