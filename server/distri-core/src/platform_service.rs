//! Unified platform service for executing distri platform actions.
//!
//! Uses a typed `PlatformAction` enum for compile-time exhaustive dispatch.
//! Works with InitializedStores so it's portable across cloud and OSS deployments.
//! Connection stores are optional — only available in cloud deployments.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use distri_stores::InitializedStores;
use distri_types::configuration::AgentConfig;
use distri_types::stores::{NewSecret, NewSkill, ThreadListFilter};

// ── Connection store traits ──────────────────────────────────
// Mirrors the traits in distri-gateway. Defined here so distri-core
// doesn't need a cross-workspace dependency. Cloud code provides
// the real implementations; tests use in-memory versions below.

/// Persistence for OAuth connection records.
#[async_trait::async_trait]
pub trait ConnectionStore: Send + Sync + 'static {
    async fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<ConnectionInfo>>;
    async fn get_by_provider(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> Result<Option<ConnectionInfo>>;
}

/// Token storage for OAuth connections.
#[async_trait::async_trait]
pub trait ConnectionTokenStore: Send + Sync + 'static {
    async fn get_token(&self, connection_id: &str) -> Result<Option<Value>>;
}

/// Minimal connection info returned by the platform service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub provider: String,
    pub status: String,
    pub scopes: Vec<String>,
}

// ── PlatformAction enum ──────────────────────────────────────

/// All platform actions as a typed enum. The tool's JSON input
/// deserializes directly into this via serde's tagged representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "params", rename_all = "snake_case")]
pub enum PlatformAction {
    // Agent management
    ListAgents,
    GetAgent { name: String },
    CreateAgent { markdown: String },

    // Skills
    ListSkills,
    GetSkill { id: String },
    CreateSkill {
        name: String,
        content: String,
        description: Option<String>,
        tags: Option<Vec<String>>,
    },
    DeleteSkill { id: String },

    // Secrets
    ListSecrets,
    GetSecret { key: String },
    SetSecret { key: String, value: String },
    DeleteSecret { key: String },
    RequestSecret { key: String, description: String },

    // Connections
    ListConnections,
    RequestConnection {
        provider: String,
        scopes: Vec<String>,
        description: String,
    },
    GetConnectionToken { provider: String },

    // Storage (cross-session memory)
    ReadStorage { key: Option<String> },
    WriteStorage { key: String, value: Value },

    // Threads
    ListThreads,
}

impl PlatformAction {
    /// Whether this action is a "stop" action that should halt the agent loop.
    pub fn is_stop_action(&self) -> bool {
        matches!(
            self,
            PlatformAction::RequestSecret { .. } | PlatformAction::RequestConnection { .. }
        )
    }
}

/// Static action metadata for skill docs and introspection.
pub const ACTIONS: &[ActionDef] = &[
    // Agents
    ActionDef { name: "list_agents", description: "List all agents in the workspace", category: "agents" },
    ActionDef { name: "get_agent", description: "Get agent details by name", category: "agents" },
    ActionDef { name: "create_agent", description: "Create a new agent from markdown definition", category: "agents" },
    // Skills
    ActionDef { name: "list_skills", description: "List available skills", category: "skills" },
    ActionDef { name: "get_skill", description: "Get skill content by ID", category: "skills" },
    ActionDef { name: "create_skill", description: "Create a new skill", category: "skills" },
    ActionDef { name: "delete_skill", description: "Delete a skill by ID", category: "skills" },
    // Secrets
    ActionDef { name: "list_secrets", description: "List secret keys (values masked)", category: "secrets" },
    ActionDef { name: "get_secret", description: "Get a secret value by key", category: "secrets" },
    ActionDef { name: "set_secret", description: "Create or update a secret", category: "secrets" },
    ActionDef { name: "delete_secret", description: "Delete a secret by key", category: "secrets" },
    ActionDef { name: "request_secret", description: "Request that a secret be configured (stops agent)", category: "secrets" },
    // Connections
    ActionDef { name: "list_connections", description: "List OAuth connections", category: "connections" },
    ActionDef { name: "request_connection", description: "Request an OAuth connection be set up (stops agent)", category: "connections" },
    ActionDef { name: "get_connection_token", description: "Get an OAuth access token for a provider", category: "connections" },
    // Storage
    ActionDef { name: "read_storage", description: "Read from persistent storage", category: "storage" },
    ActionDef { name: "write_storage", description: "Write to persistent storage", category: "storage" },
    // Threads
    ActionDef { name: "list_threads", description: "List conversation threads", category: "threads" },
];

pub struct ActionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
}

// ── DistriPlatformService ────────────────────────────────────

pub struct DistriPlatformService {
    stores: InitializedStores,
    user_id: String,
    workspace_id: Option<String>,
    connection_store: Option<Arc<dyn ConnectionStore>>,
    connection_token_store: Option<Arc<dyn ConnectionTokenStore>>,
}

impl DistriPlatformService {
    pub fn new(
        stores: InitializedStores,
        user_id: String,
        workspace_id: Option<String>,
        connection_store: Option<Arc<dyn ConnectionStore>>,
        connection_token_store: Option<Arc<dyn ConnectionTokenStore>>,
    ) -> Self {
        Self {
            stores,
            user_id,
            workspace_id,
            connection_store,
            connection_token_store,
        }
    }

    /// Execute a platform action. Returns a JSON result.
    /// Exhaustive match ensures the compiler catches missing arms.
    pub async fn execute(&self, action: PlatformAction) -> Result<Value> {
        match action {
            // Agents
            PlatformAction::ListAgents => self.list_agents().await,
            PlatformAction::GetAgent { name } => self.get_agent(&name).await,
            PlatformAction::CreateAgent { markdown } => self.create_agent(&markdown).await,

            // Skills
            PlatformAction::ListSkills => self.list_skills().await,
            PlatformAction::GetSkill { id } => self.get_skill(&id).await,
            PlatformAction::CreateSkill { name, content, description, tags } => {
                self.create_skill(&name, &content, description, tags).await
            }
            PlatformAction::DeleteSkill { id } => self.delete_skill(&id).await,

            // Secrets
            PlatformAction::ListSecrets => self.list_secrets().await,
            PlatformAction::GetSecret { key } => self.get_secret(&key).await,
            PlatformAction::SetSecret { key, value } => self.set_secret(&key, &value).await,
            PlatformAction::DeleteSecret { key } => self.delete_secret(&key).await,
            PlatformAction::RequestSecret { key, description } => {
                Ok(json!({
                    "type": "request_secret",
                    "key": key,
                    "description": description,
                }))
            }

            // Connections
            PlatformAction::ListConnections => self.list_connections().await,
            PlatformAction::RequestConnection { provider, scopes, description } => {
                Ok(json!({
                    "type": "request_connection",
                    "provider": provider,
                    "scopes": scopes,
                    "description": description,
                }))
            }
            PlatformAction::GetConnectionToken { provider } => {
                self.get_connection_token(&provider).await
            }

            // Storage
            PlatformAction::ReadStorage { key } => self.read_storage(key.as_deref()).await,
            PlatformAction::WriteStorage { key, value } => {
                self.write_storage(&key, &value).await
            }

            // Threads
            PlatformAction::ListThreads => self.list_threads().await,
        }
    }

    /// Returns metadata about all available actions (for the skill file).
    pub fn list_actions() -> Value {
        let actions: Vec<Value> = ACTIONS
            .iter()
            .map(|a| {
                json!({
                    "name": a.name,
                    "description": a.description,
                    "category": a.category,
                })
            })
            .collect();
        json!({ "actions": actions })
    }

    // ── Agent operations ──────────────────────────────────

    async fn list_agents(&self) -> Result<Value> {
        let (agents, _) = self.stores.agent_store.list(None, Some(100)).await;
        let summaries: Vec<Value> = agents
            .iter()
            .map(|config| {
                let def = config.get_definition();
                let model_name = def
                    .model_settings()
                    .map(|ms| ms.model.as_str())
                    .unwrap_or("default");
                json!({
                    "name": def.name,
                    "description": def.description,
                    "model": model_name,
                })
            })
            .collect();
        Ok(json!({ "agents": summaries }))
    }

    async fn get_agent(&self, name: &str) -> Result<Value> {
        match self.stores.agent_store.get(name).await {
            Some(config) => {
                let def = config.get_definition();
                let model_name = def
                    .model_settings()
                    .map(|ms| ms.model.as_str())
                    .unwrap_or("default");
                Ok(json!({
                    "name": def.name,
                    "description": def.description,
                    "model": model_name,
                }))
            }
            None => Err(anyhow!("Agent '{}' not found", name)),
        }
    }

    async fn create_agent(&self, markdown: &str) -> Result<Value> {
        let standard_def = distri_types::parse_agent_markdown_content(markdown)
            .await
            .map_err(|e| anyhow!("Invalid agent markdown: {}", e))?;
        let name = standard_def.name.clone();
        let config = AgentConfig::StandardAgent(standard_def);
        self.stores.agent_store.register(config).await?;
        Ok(json!({ "name": name, "created": true }))
    }

    // ── Skill operations ──────────────────────────────────

    async fn list_skills(&self) -> Result<Value> {
        let store = self
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| anyhow!("Skill store not available"))?;
        let skills = store.list_skills().await?;
        let summaries: Vec<Value> = skills
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "tags": s.tags,
                    "is_public": s.is_public,
                    "is_system": s.is_system,
                })
            })
            .collect();
        Ok(json!({ "skills": summaries }))
    }

    async fn get_skill(&self, id: &str) -> Result<Value> {
        let store = self
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| anyhow!("Skill store not available"))?;
        match store.get_skill(id).await? {
            Some(skill) => Ok(json!({
                "id": skill.id,
                "name": skill.name,
                "description": skill.description,
                "content": skill.content,
                "tags": skill.tags,
            })),
            None => Err(anyhow!("Skill '{}' not found", id)),
        }
    }

    async fn create_skill(
        &self,
        name: &str,
        content: &str,
        description: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Value> {
        let store = self
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| anyhow!("Skill store not available"))?;

        let skill = store
            .create_skill(NewSkill {
                name: name.to_string(),
                description,
                content: content.to_string(),
                tags: tags.unwrap_or_default(),
                is_public: false,
                scripts: vec![],
            })
            .await?;

        Ok(json!({ "id": skill.id, "name": skill.name, "created": true }))
    }

    async fn delete_skill(&self, id: &str) -> Result<Value> {
        let store = self
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| anyhow!("Skill store not available"))?;
        store.delete_skill(id).await?;
        Ok(json!({ "deleted": true, "id": id }))
    }

    // ── Secret operations ─────────────────────────────────

    async fn list_secrets(&self) -> Result<Value> {
        let store = self
            .stores
            .secret_store
            .as_ref()
            .ok_or_else(|| anyhow!("Secret store not available"))?;
        let secrets = store.list().await?;
        let masked: Vec<Value> = secrets
            .iter()
            .map(|s| {
                json!({ "key": s.key, "updated_at": s.updated_at.to_rfc3339() })
            })
            .collect();
        Ok(json!({ "secrets": masked }))
    }

    async fn get_secret(&self, key: &str) -> Result<Value> {
        let store = self
            .stores
            .secret_store
            .as_ref()
            .ok_or_else(|| anyhow!("Secret store not available"))?;
        match store.get(key).await? {
            Some(s) => Ok(json!({ "key": s.key, "value": s.value })),
            None => Err(anyhow!("Secret '{}' not found", key)),
        }
    }

    async fn set_secret(&self, key: &str, value: &str) -> Result<Value> {
        let store = self
            .stores
            .secret_store
            .as_ref()
            .ok_or_else(|| anyhow!("Secret store not available"))?;
        // Try create, fall back to update
        match store
            .create(NewSecret {
                key: key.to_string(),
                value: value.to_string(),
            })
            .await
        {
            Ok(_) => Ok(json!({ "key": key, "saved": true })),
            Err(_) => {
                store.update(key, value).await?;
                Ok(json!({ "key": key, "saved": true }))
            }
        }
    }

    async fn delete_secret(&self, key: &str) -> Result<Value> {
        let store = self
            .stores
            .secret_store
            .as_ref()
            .ok_or_else(|| anyhow!("Secret store not available"))?;
        store.delete(key).await?;
        Ok(json!({ "key": key, "deleted": true }))
    }

    // ── Connection operations ─────────────────────────────

    async fn list_connections(&self) -> Result<Value> {
        let store = self
            .connection_store
            .as_ref()
            .ok_or_else(|| anyhow!("Connections not available in this deployment"))?;
        let workspace_id = self
            .workspace_id
            .as_ref()
            .ok_or_else(|| anyhow!("No workspace_id available"))?;
        let connections = store.list_by_workspace(workspace_id).await?;
        let summaries: Vec<Value> = connections
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "provider": c.provider,
                    "status": c.status,
                    "scopes": c.scopes,
                })
            })
            .collect();
        Ok(json!({ "connections": summaries }))
    }

    async fn get_connection_token(&self, provider: &str) -> Result<Value> {
        let conn_store = self
            .connection_store
            .as_ref()
            .ok_or_else(|| anyhow!("Connections not available in this deployment"))?;
        let token_store = self
            .connection_token_store
            .as_ref()
            .ok_or_else(|| anyhow!("Connection token store not available"))?;
        let workspace_id = self
            .workspace_id
            .as_ref()
            .ok_or_else(|| anyhow!("No workspace_id available"))?;

        let connection = conn_store
            .get_by_provider(workspace_id, provider)
            .await?
            .ok_or_else(|| anyhow!("No connection found for provider '{}'", provider))?;

        if connection.status != "connected" {
            return Err(anyhow!(
                "Connection for '{}' is not active (status: {})",
                provider,
                connection.status
            ));
        }

        let token = token_store
            .get_token(&connection.id)
            .await?
            .ok_or_else(|| anyhow!("No token available for provider '{}'", provider))?;

        Ok(json!({
            "provider": provider,
            "connection_id": connection.id,
            "token": token,
        }))
    }

    // ── Storage operations ────────────────────────────────

    async fn read_storage(&self, key: Option<&str>) -> Result<Value> {
        let namespace = format!("platform_storage:{}", self.user_id);
        if let Some(key) = key {
            let value = self
                .stores
                .session_store
                .get_value(&namespace, key)
                .await?;
            Ok(json!({ "key": key, "value": value }))
        } else {
            let all = self
                .stores
                .session_store
                .get_all_values(&namespace)
                .await?;
            Ok(json!({ "storage": all }))
        }
    }

    async fn write_storage(&self, key: &str, value: &Value) -> Result<Value> {
        let namespace = format!("platform_storage:{}", self.user_id);
        self.stores
            .session_store
            .set_value(&namespace, key, value)
            .await?;
        Ok(json!({ "key": key, "stored": true }))
    }

    // ── Thread operations ─────────────────────────────────

    async fn list_threads(&self) -> Result<Value> {
        let filter = ThreadListFilter::default();
        let response = self
            .stores
            .thread_store
            .list_threads(&filter, Some(20), None)
            .await?;
        let summaries: Vec<Value> = response
            .threads
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "title": t.title,
                    "agent_id": t.agent_id,
                    "updated_at": t.updated_at,
                })
            })
            .collect();
        Ok(json!({ "threads": summaries }))
    }
}

// ── DistriPlatformTool ────────────────────────────────────

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};

/// Unified platform tool that dispatches to DistriPlatformService.
#[derive(Debug)]
pub struct DistriPlatformTool;

/// Stop-action message prefix for the LLM
const STOP_MSG_REQUEST_SECRET: &str =
    "I need a secret to be configured before I can continue. \
     Please set it up and send me a message when ready.";
const STOP_MSG_REQUEST_CONNECTION: &str =
    "I need an OAuth connection to be set up before I can continue. \
     Please configure it in /settings/connections and send me a message when ready.";

#[async_trait::async_trait]
impl Tool for DistriPlatformTool {
    fn get_name(&self) -> String {
        "distri_platform".to_string()
    }

    fn get_description(&self) -> String {
        "Execute platform actions (manage agents, skills, secrets, storage, threads, connections). \
         Load the distri_platform skill first to see available actions and their parameters."
            .to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action name (e.g. list_agents, create_skill, set_secret, list_connections)"
                },
                "params": {
                    "type": "object",
                    "description": "Action parameters (varies by action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        _: ToolCall,
        _: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DistriPlatformTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;

        // Deserialize typed action from tool input
        let action: PlatformAction = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| AgentError::ToolExecution(format!("Invalid platform action: {}", e)))?;

        let is_stop = action.is_stop_action();

        let service = DistriPlatformService::new(
            orchestrator.stores.clone(),
            context.user_id.clone(),
            context.workspace_id.clone(),
            None, // Connection stores injected by cloud layer
            None,
        );

        match service.execute(action).await {
            Ok(result) => {
                if is_stop {
                    // For stop actions, return both the data (for UI) and a text
                    // message (for the LLM to see and decide to stop).
                    let stop_msg = match result.get("type").and_then(|t| t.as_str()) {
                        Some("request_secret") => format!(
                            "{}\n\nSecret needed: \"{}\" - {}",
                            STOP_MSG_REQUEST_SECRET,
                            result["key"].as_str().unwrap_or("unknown"),
                            result["description"].as_str().unwrap_or(""),
                        ),
                        Some("request_connection") => format!(
                            "{}\n\nProvider: {}, Scopes: {:?} - {}",
                            STOP_MSG_REQUEST_CONNECTION,
                            result["provider"].as_str().unwrap_or("unknown"),
                            result["scopes"],
                            result["description"].as_str().unwrap_or(""),
                        ),
                        _ => "Action requires user input. Please stop and wait.".to_string(),
                    };
                    Ok(vec![Part::Data(result), Part::Text(stop_msg)])
                } else {
                    Ok(vec![Part::Data(result)])
                }
            }
            Err(e) => Err(AgentError::ToolExecution(e.to_string())),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PlatformAction serde tests ────────────────────────

    #[test]
    fn test_deserialize_list_agents() {
        let input = json!({"action": "list_agents"});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::ListAgents));
    }

    #[test]
    fn test_deserialize_get_agent() {
        let input = json!({"action": "get_agent", "params": {"name": "my_agent"}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::GetAgent { name } if name == "my_agent"));
    }

    #[test]
    fn test_deserialize_create_agent() {
        let input = json!({"action": "create_agent", "params": {"markdown": "---\nname = \"test\"\n---\nHello"}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::CreateAgent { .. }));
    }

    #[test]
    fn test_deserialize_create_skill() {
        let input = json!({
            "action": "create_skill",
            "params": {
                "name": "test_skill",
                "content": "# Test",
                "description": "A test skill",
                "tags": ["test", "demo"]
            }
        });
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        match action {
            PlatformAction::CreateSkill { name, content, description, tags } => {
                assert_eq!(name, "test_skill");
                assert_eq!(content, "# Test");
                assert_eq!(description, Some("A test skill".to_string()));
                assert_eq!(tags, Some(vec!["test".to_string(), "demo".to_string()]));
            }
            _ => panic!("Expected CreateSkill"),
        }
    }

    #[test]
    fn test_deserialize_create_skill_minimal() {
        let input = json!({
            "action": "create_skill",
            "params": { "name": "minimal", "content": "hello" }
        });
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        match action {
            PlatformAction::CreateSkill { name, description, tags, .. } => {
                assert_eq!(name, "minimal");
                assert!(description.is_none());
                assert!(tags.is_none());
            }
            _ => panic!("Expected CreateSkill"),
        }
    }

    #[test]
    fn test_deserialize_set_secret() {
        let input = json!({"action": "set_secret", "params": {"key": "API_KEY", "value": "abc123"}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::SetSecret { key, value } if key == "API_KEY" && value == "abc123"));
    }

    #[test]
    fn test_deserialize_request_secret() {
        let input = json!({
            "action": "request_secret",
            "params": {"key": "OPENAI_KEY", "description": "Need OpenAI API key"}
        });
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::RequestSecret { .. }));
    }

    #[test]
    fn test_deserialize_request_connection() {
        let input = json!({
            "action": "request_connection",
            "params": {
                "provider": "google",
                "scopes": ["drive.readonly"],
                "description": "Need Google Drive access"
            }
        });
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        match action {
            PlatformAction::RequestConnection { provider, scopes, description } => {
                assert_eq!(provider, "google");
                assert_eq!(scopes, vec!["drive.readonly"]);
                assert_eq!(description, "Need Google Drive access");
            }
            _ => panic!("Expected RequestConnection"),
        }
    }

    #[test]
    fn test_deserialize_get_connection_token() {
        let input = json!({"action": "get_connection_token", "params": {"provider": "google"}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::GetConnectionToken { provider } if provider == "google"));
    }

    #[test]
    fn test_deserialize_read_storage_with_key() {
        let input = json!({"action": "read_storage", "params": {"key": "mykey"}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::ReadStorage { key: Some(k) } if k == "mykey"));
    }

    #[test]
    fn test_deserialize_read_storage_no_key() {
        let input = json!({"action": "read_storage", "params": {}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::ReadStorage { key: None }));
    }

    #[test]
    fn test_deserialize_write_storage() {
        let input = json!({"action": "write_storage", "params": {"key": "k", "value": {"nested": true}}});
        let action: PlatformAction = serde_json::from_value(input).unwrap();
        assert!(matches!(action, PlatformAction::WriteStorage { .. }));
    }

    #[test]
    fn test_deserialize_unknown_action_fails() {
        let input = json!({"action": "nonexistent"});
        let result: Result<PlatformAction, _> = serde_json::from_value(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_stop_action() {
        assert!(PlatformAction::RequestSecret {
            key: "k".into(),
            description: "d".into()
        }.is_stop_action());

        assert!(PlatformAction::RequestConnection {
            provider: "google".into(),
            scopes: vec![],
            description: "d".into(),
        }.is_stop_action());

        assert!(!PlatformAction::ListAgents.is_stop_action());
        assert!(!PlatformAction::GetConnectionToken { provider: "x".into() }.is_stop_action());
    }

    // ── list_actions metadata ─────────────────────────────

    #[test]
    fn test_list_actions() {
        let actions = DistriPlatformService::list_actions();
        assert!(actions["actions"].is_array());
        let arr = actions["actions"].as_array().unwrap();
        assert!(arr.len() >= 18, "Expected at least 18 actions, got {}", arr.len());
        // Check new actions exist
        assert!(arr.iter().any(|a| a["name"] == "create_agent"));
        assert!(arr.iter().any(|a| a["name"] == "request_secret"));
        assert!(arr.iter().any(|a| a["name"] == "list_connections"));
        assert!(arr.iter().any(|a| a["name"] == "request_connection"));
        assert!(arr.iter().any(|a| a["name"] == "get_connection_token"));
    }

    #[test]
    fn test_list_actions_categories() {
        let actions = DistriPlatformService::list_actions();
        let arr = actions["actions"].as_array().unwrap();
        let categories: Vec<&str> = arr
            .iter()
            .filter_map(|a| a["category"].as_str())
            .collect();
        assert!(categories.contains(&"agents"));
        assert!(categories.contains(&"skills"));
        assert!(categories.contains(&"secrets"));
        assert!(categories.contains(&"connections"));
        assert!(categories.contains(&"storage"));
        assert!(categories.contains(&"threads"));
    }

    #[test]
    fn test_actions_have_descriptions() {
        let actions = DistriPlatformService::list_actions();
        let arr = actions["actions"].as_array().unwrap();
        for action in arr {
            assert!(
                action["description"].as_str().is_some(),
                "Action {:?} missing description",
                action["name"]
            );
            assert!(
                !action["description"].as_str().unwrap().is_empty(),
                "Action {:?} has empty description",
                action["name"]
            );
        }
    }

    #[test]
    fn test_unknown_action_not_in_registry() {
        let actions = DistriPlatformService::list_actions();
        let names: Vec<&str> = actions["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(!names.contains(&"nonexistent_action"));
    }

    // ── Integration tests with in-memory stores ───────────

    use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

    fn test_store_config() -> StoreConfig {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
        StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn create_test_service() -> DistriPlatformService {
        let stores = distri_stores::initialize_stores(&test_store_config()).await.unwrap();
        DistriPlatformService::new(
            stores,
            "test_user".to_string(),
            None,
            None,
            None,
        )
    }

    async fn create_test_service_with_connections(
        conn_store: Arc<dyn ConnectionStore>,
        token_store: Arc<dyn ConnectionTokenStore>,
    ) -> DistriPlatformService {
        let stores = distri_stores::initialize_stores(&test_store_config()).await.unwrap();
        DistriPlatformService::new(
            stores,
            "test_user".to_string(),
            Some("workspace_1".to_string()),
            Some(conn_store),
            Some(token_store),
        )
    }

    // In-memory connection stores for tests
    struct TestConnectionStore {
        connections: tokio::sync::RwLock<Vec<ConnectionInfo>>,
    }

    impl TestConnectionStore {
        fn new() -> Self {
            Self { connections: tokio::sync::RwLock::new(Vec::new()) }
        }

        async fn add(&self, info: ConnectionInfo) {
            self.connections.write().await.push(info);
        }
    }

    #[async_trait::async_trait]
    impl ConnectionStore for TestConnectionStore {
        async fn list_by_workspace(&self, _workspace_id: &str) -> Result<Vec<ConnectionInfo>> {
            Ok(self.connections.read().await.clone())
        }
        async fn get_by_provider(&self, _workspace_id: &str, provider: &str) -> Result<Option<ConnectionInfo>> {
            let conns = self.connections.read().await;
            Ok(conns.iter().find(|c| c.provider == provider).cloned())
        }
    }

    struct TestConnectionTokenStore {
        tokens: tokio::sync::RwLock<std::collections::HashMap<String, Value>>,
    }

    impl TestConnectionTokenStore {
        fn new() -> Self {
            Self { tokens: tokio::sync::RwLock::new(std::collections::HashMap::new()) }
        }

        async fn set_token(&self, connection_id: &str, token: Value) {
            self.tokens.write().await.insert(connection_id.to_string(), token);
        }
    }

    #[async_trait::async_trait]
    impl ConnectionTokenStore for TestConnectionTokenStore {
        async fn get_token(&self, connection_id: &str) -> Result<Option<Value>> {
            Ok(self.tokens.read().await.get(connection_id).cloned())
        }
    }

    // ── Agent tests ───────────────────────────────────────

    #[tokio::test]
    async fn test_list_agents_empty() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::ListAgents).await.unwrap();
        assert!(result["agents"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_agent_not_found() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::GetAgent { name: "ghost".into() }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_create_agent_valid() {
        let svc = create_test_service().await;
        let markdown = r#"---
name = "test_bot"
description = "A test bot"
---
You are a helpful test bot.
"#;
        let result = svc.execute(PlatformAction::CreateAgent { markdown: markdown.into() }).await.unwrap();
        assert_eq!(result["name"], "test_bot");
        assert_eq!(result["created"], true);

        // Verify agent is now listed
        let list = svc.execute(PlatformAction::ListAgents).await.unwrap();
        let agents = list["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["name"], "test_bot");
    }

    #[tokio::test]
    async fn test_create_agent_invalid_markdown() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::CreateAgent {
            markdown: "no frontmatter here".into()
        }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid agent markdown"));
    }

    #[tokio::test]
    async fn test_create_then_get_agent() {
        let svc = create_test_service().await;
        let markdown = r#"---
name = "fetcher"
description = "Fetches stuff"
---
You fetch things.
"#;
        svc.execute(PlatformAction::CreateAgent { markdown: markdown.into() }).await.unwrap();
        let result = svc.execute(PlatformAction::GetAgent { name: "fetcher".into() }).await.unwrap();
        assert_eq!(result["name"], "fetcher");
        assert_eq!(result["description"], "Fetches stuff");
    }

    // ── Storage tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_write_then_read_storage() {
        let svc = create_test_service().await;
        svc.execute(PlatformAction::WriteStorage {
            key: "color".into(),
            value: json!("blue"),
        }).await.unwrap();

        let result = svc.execute(PlatformAction::ReadStorage { key: Some("color".into()) }).await.unwrap();
        assert_eq!(result["value"], "blue");
    }

    #[tokio::test]
    async fn test_read_storage_missing_key() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::ReadStorage { key: Some("missing".into()) }).await.unwrap();
        assert!(result["value"].is_null());
    }

    #[tokio::test]
    async fn test_read_storage_list_all() {
        let svc = create_test_service().await;
        svc.execute(PlatformAction::WriteStorage { key: "a".into(), value: json!(1) }).await.unwrap();
        svc.execute(PlatformAction::WriteStorage { key: "b".into(), value: json!(2) }).await.unwrap();

        let result = svc.execute(PlatformAction::ReadStorage { key: None }).await.unwrap();
        let storage = result["storage"].as_object().unwrap();
        assert_eq!(storage.len(), 2);
    }

    #[tokio::test]
    async fn test_write_storage_complex_value() {
        let svc = create_test_service().await;
        let complex = json!({"nested": {"array": [1, 2, 3]}});
        svc.execute(PlatformAction::WriteStorage { key: "data".into(), value: complex.clone() }).await.unwrap();

        let result = svc.execute(PlatformAction::ReadStorage { key: Some("data".into()) }).await.unwrap();
        assert_eq!(result["value"], complex);
    }

    // ── Connection tests ──────────────────────────────────

    #[tokio::test]
    async fn test_list_connections_no_store() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::ListConnections).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not available"));
    }

    #[tokio::test]
    async fn test_get_connection_token_no_store() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::GetConnectionToken { provider: "google".into() }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not available"));
    }

    #[tokio::test]
    async fn test_list_connections_with_store() {
        let conn_store = Arc::new(TestConnectionStore::new());
        conn_store.add(ConnectionInfo {
            id: "conn_1".into(),
            provider: "google".into(),
            status: "connected".into(),
            scopes: vec!["drive.readonly".into()],
        }).await;
        let token_store = Arc::new(TestConnectionTokenStore::new());

        let svc = create_test_service_with_connections(conn_store, token_store).await;
        let result = svc.execute(PlatformAction::ListConnections).await.unwrap();
        let conns = result["connections"].as_array().unwrap();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0]["provider"], "google");
        assert_eq!(conns[0]["status"], "connected");
    }

    #[tokio::test]
    async fn test_get_connection_token_success() {
        let conn_store = Arc::new(TestConnectionStore::new());
        conn_store.add(ConnectionInfo {
            id: "conn_1".into(),
            provider: "google".into(),
            status: "connected".into(),
            scopes: vec!["drive.readonly".into()],
        }).await;
        let token_store = Arc::new(TestConnectionTokenStore::new());
        token_store.set_token("conn_1", json!({"access_token": "ya29.xxx"})).await;

        let svc = create_test_service_with_connections(conn_store, token_store).await;
        let result = svc.execute(PlatformAction::GetConnectionToken { provider: "google".into() }).await.unwrap();
        assert_eq!(result["provider"], "google");
        assert_eq!(result["token"]["access_token"], "ya29.xxx");
    }

    #[tokio::test]
    async fn test_get_connection_token_not_connected() {
        let conn_store = Arc::new(TestConnectionStore::new());
        conn_store.add(ConnectionInfo {
            id: "conn_1".into(),
            provider: "google".into(),
            status: "expired".into(),
            scopes: vec![],
        }).await;
        let token_store = Arc::new(TestConnectionTokenStore::new());

        let svc = create_test_service_with_connections(conn_store, token_store).await;
        let result = svc.execute(PlatformAction::GetConnectionToken { provider: "google".into() }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not active"));
    }

    #[tokio::test]
    async fn test_get_connection_token_no_token() {
        let conn_store = Arc::new(TestConnectionStore::new());
        conn_store.add(ConnectionInfo {
            id: "conn_1".into(),
            provider: "google".into(),
            status: "connected".into(),
            scopes: vec![],
        }).await;
        let token_store = Arc::new(TestConnectionTokenStore::new());
        // No token stored

        let svc = create_test_service_with_connections(conn_store, token_store).await;
        let result = svc.execute(PlatformAction::GetConnectionToken { provider: "google".into() }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No token"));
    }

    #[tokio::test]
    async fn test_get_connection_token_provider_not_found() {
        let conn_store = Arc::new(TestConnectionStore::new());
        let token_store = Arc::new(TestConnectionTokenStore::new());

        let svc = create_test_service_with_connections(conn_store, token_store).await;
        let result = svc.execute(PlatformAction::GetConnectionToken { provider: "github".into() }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No connection found"));
    }

    // ── Stop action result tests ──────────────────────────

    #[tokio::test]
    async fn test_request_secret_returns_data() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::RequestSecret {
            key: "OPENAI_KEY".into(),
            description: "Need OpenAI API key for completions".into(),
        }).await.unwrap();
        assert_eq!(result["type"], "request_secret");
        assert_eq!(result["key"], "OPENAI_KEY");
        assert_eq!(result["description"], "Need OpenAI API key for completions");
    }

    #[tokio::test]
    async fn test_request_connection_returns_data() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::RequestConnection {
            provider: "google".into(),
            scopes: vec!["drive.readonly".into(), "sheets.readonly".into()],
            description: "Need Google access to list sheets".into(),
        }).await.unwrap();
        assert_eq!(result["type"], "request_connection");
        assert_eq!(result["provider"], "google");
        let scopes = result["scopes"].as_array().unwrap();
        assert_eq!(scopes.len(), 2);
    }

    // ── Thread tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_list_threads_empty() {
        let svc = create_test_service().await;
        let result = svc.execute(PlatformAction::ListThreads).await.unwrap();
        assert!(result["threads"].as_array().unwrap().is_empty());
    }

    // ── Roundtrip serialize/deserialize ───────────────────

    #[test]
    fn test_all_action_variants_roundtrip() {
        let actions = vec![
            json!({"action": "list_agents"}),
            json!({"action": "get_agent", "params": {"name": "x"}}),
            json!({"action": "create_agent", "params": {"markdown": "---\nname=\"x\"\n---\nhi"}}),
            json!({"action": "list_skills"}),
            json!({"action": "get_skill", "params": {"id": "s1"}}),
            json!({"action": "create_skill", "params": {"name": "s", "content": "c"}}),
            json!({"action": "delete_skill", "params": {"id": "s1"}}),
            json!({"action": "list_secrets"}),
            json!({"action": "get_secret", "params": {"key": "k"}}),
            json!({"action": "set_secret", "params": {"key": "k", "value": "v"}}),
            json!({"action": "delete_secret", "params": {"key": "k"}}),
            json!({"action": "request_secret", "params": {"key": "k", "description": "d"}}),
            json!({"action": "list_connections"}),
            json!({"action": "request_connection", "params": {"provider": "p", "scopes": ["s"], "description": "d"}}),
            json!({"action": "get_connection_token", "params": {"provider": "p"}}),
            json!({"action": "read_storage", "params": {}}),
            json!({"action": "read_storage", "params": {"key": "k"}}),
            json!({"action": "write_storage", "params": {"key": "k", "value": 42}}),
            json!({"action": "list_threads"}),
        ];

        for (i, input) in actions.iter().enumerate() {
            let result: Result<PlatformAction, _> = serde_json::from_value(input.clone());
            assert!(result.is_ok(), "Action {} failed to deserialize: {:?} - {:?}", i, input, result.err());
        }
    }
}
