//! Unified platform service for executing distri platform actions.
//!
//! Provides a single entry point (execute) that dispatches to the right
//! store operation based on the action name. Works with InitializedStores
//! so it's portable across cloud and OSS deployments.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use distri_stores::InitializedStores;
use distri_types::stores::{NewSecret, NewSkill, ThreadListFilter};

/// All supported platform actions
pub const ACTIONS: &[ActionDef] = &[
    // Meta
    ActionDef {
        name: "list_actions",
        description: "List all available platform actions",
        category: "meta",
    },
    // Agents
    ActionDef {
        name: "list_agents",
        description: "List all agents in the workspace",
        category: "agents",
    },
    ActionDef {
        name: "get_agent",
        description: "Get agent details by name or ID",
        category: "agents",
    },
    // Skills
    ActionDef {
        name: "list_skills",
        description: "List available skills",
        category: "skills",
    },
    ActionDef {
        name: "get_skill",
        description: "Get skill content by ID or name",
        category: "skills",
    },
    ActionDef {
        name: "create_skill",
        description: "Create a new skill",
        category: "skills",
    },
    ActionDef {
        name: "delete_skill",
        description: "Delete a skill by ID",
        category: "skills",
    },
    // Secrets
    ActionDef {
        name: "list_secrets",
        description: "List secret keys (values masked)",
        category: "secrets",
    },
    ActionDef {
        name: "get_secret",
        description: "Get a secret value by key",
        category: "secrets",
    },
    ActionDef {
        name: "set_secret",
        description: "Create or update a secret",
        category: "secrets",
    },
    ActionDef {
        name: "delete_secret",
        description: "Delete a secret by key",
        category: "secrets",
    },
    // Storage (persistent key-value)
    ActionDef {
        name: "read_storage",
        description: "Read from persistent storage",
        category: "storage",
    },
    ActionDef {
        name: "write_storage",
        description: "Write to persistent storage",
        category: "storage",
    },
    // Threads
    ActionDef {
        name: "list_threads",
        description: "List conversation threads",
        category: "threads",
    },
    // Connections
    ActionDef {
        name: "list_connections",
        description: "List connected integrations (OAuth providers)",
        category: "connections",
    },
    ActionDef {
        name: "get_connection_token",
        description: "Get a valid access token for a connected provider",
        category: "connections",
    },
];

pub struct ActionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
}

/// Abstraction for connection operations.
/// Cloud layer injects an implementation; OSS deployments pass None.
#[async_trait::async_trait]
pub trait PlatformConnectionStore: Send + Sync {
    /// List all connections for a workspace. Returns JSON array of connection summaries.
    async fn list_connections(&self, workspace_id: &str) -> Result<Value>;
    /// Get a valid access token for a connected provider. Auto-refreshes if expired.
    async fn get_connection_token(&self, workspace_id: &str, provider: &str) -> Result<Value>;
}

pub struct DistriPlatformService {
    stores: InitializedStores,
    user_id: String,
    workspace_id: Option<String>,
    connection_store: Option<Arc<dyn PlatformConnectionStore>>,
}

impl DistriPlatformService {
    pub fn new(
        stores: InitializedStores,
        user_id: String,
        workspace_id: Option<String>,
        connection_store: Option<Arc<dyn PlatformConnectionStore>>,
    ) -> Self {
        Self {
            stores,
            user_id,
            workspace_id,
            connection_store,
        }
    }

    /// Execute a platform action by name with JSON params.
    /// Returns a JSON result.
    pub async fn execute(&self, action: &str, params: &Value) -> Result<Value> {
        match action {
            // Meta
            "list_actions" => Ok(Self::list_actions()),

            // Agents
            "list_agents" => self.list_agents().await,
            "get_agent" => {
                let name = params["name"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'name' parameter"))?;
                self.get_agent(name).await
            }

            // Skills
            "list_skills" => self.list_skills().await,
            "get_skill" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'id' parameter"))?;
                self.get_skill(id).await
            }
            "create_skill" => self.create_skill(params).await,
            "delete_skill" => {
                let id = params["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'id' parameter"))?;
                self.delete_skill(id).await
            }

            // Secrets
            "list_secrets" => self.list_secrets().await,
            "get_secret" => {
                let key = params["key"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'key' parameter"))?;
                self.get_secret(key).await
            }
            "set_secret" => {
                let key = params["key"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'key' parameter"))?;
                let value = params["value"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'value' parameter"))?;
                self.set_secret(key, value).await
            }
            "delete_secret" => {
                let key = params["key"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'key' parameter"))?;
                self.delete_secret(key).await
            }

            // Storage
            "read_storage" => {
                let key = params["key"].as_str();
                self.read_storage(key).await
            }
            "write_storage" => {
                let key = params["key"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'key' parameter"))?;
                let value = &params["value"];
                if value.is_null() {
                    return Err(anyhow!("Missing 'value' parameter"));
                }
                self.write_storage(key, value).await
            }

            // Threads
            "list_threads" => self.list_threads().await,

            // Connections
            "list_connections" => self.list_connections().await,
            "get_connection_token" => {
                let provider = params["provider"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'provider' parameter"))?;
                self.get_connection_token(provider).await
            }

            _ => Err(anyhow!(
                "Unknown action: '{}'. Use list_actions to see available actions.",
                action
            )),
        }
    }

    /// Returns metadata about all available actions (for the skill file)
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

    async fn create_skill(&self, params: &Value) -> Result<Value> {
        let store = self
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| anyhow!("Skill store not available"))?;
        let name = params["name"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing 'name'"))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing 'content'"))?;
        let description = params["description"].as_str().map(|s| s.to_string());
        let tags: Vec<String> = params["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let skill = store
            .create_skill(NewSkill {
                name: name.to_string(),
                description,
                content: content.to_string(),
                tags,
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

    // ── Connection operations ──────────────────────────────

    async fn list_connections(&self) -> Result<Value> {
        let store = self
            .connection_store
            .as_ref()
            .ok_or_else(|| anyhow!("Connections not available in this deployment"))?;
        let workspace_id = self
            .workspace_id
            .as_deref()
            .ok_or_else(|| anyhow!("No workspace context available"))?;
        store.list_connections(workspace_id).await
    }

    async fn get_connection_token(&self, provider: &str) -> Result<Value> {
        let store = self
            .connection_store
            .as_ref()
            .ok_or_else(|| anyhow!("Connections not available in this deployment"))?;
        let workspace_id = self
            .workspace_id
            .as_deref()
            .ok_or_else(|| anyhow!("No workspace context available"))?;
        store.get_connection_token(workspace_id, provider).await
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

use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};

/// Unified platform tool that dispatches to DistriPlatformService.
#[derive(Debug)]
pub struct DistriPlatformTool;

#[async_trait::async_trait]
impl Tool for DistriPlatformTool {
    fn get_name(&self) -> String {
        "distri_platform".to_string()
    }

    fn get_description(&self) -> String {
        "Execute platform actions (manage agents, skills, secrets, storage, threads). \
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
                    "description": "Action name (e.g. list_agents, create_skill, set_secret)"
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
        let action = tool_call.input["action"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecution("Missing 'action' parameter".to_string()))?;
        let params = tool_call.input.get("params").cloned().unwrap_or(json!({}));

        let service = DistriPlatformService::new(
            orchestrator.stores.clone(),
            context.user_id.clone(),
            context.workspace_id.clone(),
            context.connection_store.clone(),
        );

        match service.execute(action, &params).await {
            Ok(result) => Ok(vec![Part::Data(result)]),
            Err(e) => Err(AgentError::ToolExecution(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_stores::initialize_stores;
    use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

    // ── Helpers ─────────────────────────────────────────────

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

    async fn make_service() -> DistriPlatformService {
        let stores = initialize_stores(&test_store_config()).await.unwrap();
        DistriPlatformService::new(stores, "test-user".to_string(), None, None)
    }

    /// In-memory PlatformConnectionStore for testing
    struct MockConnectionStore {
        connections: Vec<Value>,
        tokens: std::collections::HashMap<String, Value>,
    }

    impl MockConnectionStore {
        fn new() -> Self {
            Self {
                connections: vec![],
                tokens: std::collections::HashMap::new(),
            }
        }

        fn with_connection(mut self, name: &str, status: &str) -> Self {
            self.connections.push(json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "name": name,
                "status": status,
                "config": {},
                "created_at": chrono::Utc::now().to_rfc3339(),
            }));
            self
        }

        fn with_token(mut self, provider: &str, access_token: &str) -> Self {
            self.tokens.insert(
                provider.to_string(),
                json!({
                    "provider": provider,
                    "access_token": access_token,
                    "token_type": "Bearer",
                }),
            );
            self
        }
    }

    #[async_trait::async_trait]
    impl PlatformConnectionStore for MockConnectionStore {
        async fn list_connections(&self, _workspace_id: &str) -> Result<Value> {
            Ok(json!({ "connections": self.connections }))
        }
        async fn get_connection_token(&self, _workspace_id: &str, provider: &str) -> Result<Value> {
            self.tokens
                .get(provider)
                .cloned()
                .ok_or_else(|| anyhow!("No token for provider '{}'", provider))
        }
    }

    // ── Static / list_actions tests ─────────────────────────

    #[test]
    fn test_list_actions_returns_all_actions() {
        let actions = DistriPlatformService::list_actions();
        let arr = actions["actions"].as_array().unwrap();
        // Must have all ACTIONS entries
        assert_eq!(arr.len(), ACTIONS.len());
        // Check known actions exist
        let names: Vec<&str> = arr.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"list_actions"));
        assert!(names.contains(&"list_agents"));
        assert!(names.contains(&"get_agent"));
        assert!(names.contains(&"list_skills"));
        assert!(names.contains(&"get_skill"));
        assert!(names.contains(&"create_skill"));
        assert!(names.contains(&"delete_skill"));
        assert!(names.contains(&"list_secrets"));
        assert!(names.contains(&"get_secret"));
        assert!(names.contains(&"set_secret"));
        assert!(names.contains(&"delete_secret"));
        assert!(names.contains(&"read_storage"));
        assert!(names.contains(&"write_storage"));
        assert!(names.contains(&"list_threads"));
        assert!(names.contains(&"list_connections"));
        assert!(names.contains(&"get_connection_token"));
    }

    #[test]
    fn test_list_actions_categories() {
        let actions = DistriPlatformService::list_actions();
        let arr = actions["actions"].as_array().unwrap();
        let categories: std::collections::HashSet<&str> = arr
            .iter()
            .filter_map(|a| a["category"].as_str())
            .collect();
        assert!(categories.contains("meta"));
        assert!(categories.contains("agents"));
        assert!(categories.contains("skills"));
        assert!(categories.contains("secrets"));
        assert!(categories.contains("storage"));
        assert!(categories.contains("threads"));
        assert!(categories.contains("connections"));
    }

    #[test]
    fn test_actions_have_descriptions() {
        for action in ACTIONS {
            assert!(
                !action.description.is_empty(),
                "Action '{}' has empty description",
                action.name
            );
        }
    }

    // ── list_actions as routable action ─────────────────────

    #[tokio::test]
    async fn test_execute_list_actions() {
        let svc = make_service().await;
        let result = svc.execute("list_actions", &json!({})).await.unwrap();
        assert!(result["actions"].is_array());
        let arr = result["actions"].as_array().unwrap();
        assert!(arr.iter().any(|a| a["name"] == "list_agents"));
    }

    // ── Unknown action ──────────────────────────────────────

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let svc = make_service().await;
        let err = svc.execute("nonexistent", &json!({})).await.unwrap_err();
        assert!(err.to_string().contains("Unknown action"));
        assert!(err.to_string().contains("nonexistent"));
    }

    // ── Agent operations ────────────────────────────────────

    #[tokio::test]
    async fn test_list_agents_empty() {
        let svc = make_service().await;
        let result = svc.execute("list_agents", &json!({})).await.unwrap();
        assert!(result["agents"].is_array());
    }

    #[tokio::test]
    async fn test_get_agent_not_found() {
        let svc = make_service().await;
        let err = svc
            .execute("get_agent", &json!({"name": "nonexistent"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_get_agent_missing_param() {
        let svc = make_service().await;
        let err = svc.execute("get_agent", &json!({})).await.unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    // ── Skill operations ────────────────────────────────────

    #[tokio::test]
    async fn test_list_skills_empty() {
        let svc = make_service().await;
        let result = svc.execute("list_skills", &json!({})).await.unwrap();
        assert!(result["skills"].is_array());
        assert_eq!(result["skills"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_create_and_get_skill() {
        let svc = make_service().await;
        let result = svc
            .execute(
                "create_skill",
                &json!({
                    "name": "test-skill",
                    "content": "# Test Skill\nDo something",
                    "description": "A test skill",
                    "tags": ["test", "example"]
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["created"], true);
        let id = result["id"].as_str().unwrap();

        // Get it back
        let skill = svc
            .execute("get_skill", &json!({"id": id}))
            .await
            .unwrap();
        assert_eq!(skill["name"], "test-skill");
        assert_eq!(skill["content"], "# Test Skill\nDo something");
    }

    #[tokio::test]
    async fn test_create_skill_missing_name() {
        let svc = make_service().await;
        let err = svc
            .execute("create_skill", &json!({"content": "hello"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_create_skill_missing_content() {
        let svc = make_service().await;
        let err = svc
            .execute("create_skill", &json!({"name": "test"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_delete_skill() {
        let svc = make_service().await;
        let created = svc
            .execute(
                "create_skill",
                &json!({"name": "to-delete", "content": "bye"}),
            )
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap();

        let result = svc
            .execute("delete_skill", &json!({"id": id}))
            .await
            .unwrap();
        assert_eq!(result["deleted"], true);

        // Verify it's gone
        let err = svc
            .execute("get_skill", &json!({"id": id}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_skills_after_create() {
        let svc = make_service().await;
        svc.execute(
            "create_skill",
            &json!({"name": "skill-1", "content": "content"}),
        )
        .await
        .unwrap();
        svc.execute(
            "create_skill",
            &json!({"name": "skill-2", "content": "content"}),
        )
        .await
        .unwrap();

        let result = svc.execute("list_skills", &json!({})).await.unwrap();
        assert_eq!(result["skills"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_get_skill_not_found() {
        let svc = make_service().await;
        let err = svc
            .execute("get_skill", &json!({"id": "nonexistent"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    // ── Secret operations ───────────────────────────────────

    #[tokio::test]
    async fn test_list_secrets_empty() {
        let svc = make_service().await;
        let result = svc.execute("list_secrets", &json!({})).await.unwrap();
        assert!(result["secrets"].is_array());
        assert_eq!(result["secrets"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_set_and_get_secret() {
        let svc = make_service().await;
        let result = svc
            .execute("set_secret", &json!({"key": "API_KEY", "value": "sk-123"}))
            .await
            .unwrap();
        assert_eq!(result["saved"], true);

        let secret = svc
            .execute("get_secret", &json!({"key": "API_KEY"}))
            .await
            .unwrap();
        assert_eq!(secret["key"], "API_KEY");
        assert_eq!(secret["value"], "sk-123");
    }

    #[tokio::test]
    async fn test_set_secret_update_existing() {
        let svc = make_service().await;
        svc.execute("set_secret", &json!({"key": "KEY", "value": "v1"}))
            .await
            .unwrap();
        svc.execute("set_secret", &json!({"key": "KEY", "value": "v2"}))
            .await
            .unwrap();

        let secret = svc
            .execute("get_secret", &json!({"key": "KEY"}))
            .await
            .unwrap();
        assert_eq!(secret["value"], "v2");
    }

    #[tokio::test]
    async fn test_delete_secret() {
        let svc = make_service().await;
        svc.execute("set_secret", &json!({"key": "TEMP", "value": "val"}))
            .await
            .unwrap();
        let result = svc
            .execute("delete_secret", &json!({"key": "TEMP"}))
            .await
            .unwrap();
        assert_eq!(result["deleted"], true);

        let err = svc
            .execute("get_secret", &json!({"key": "TEMP"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_get_secret_not_found() {
        let svc = make_service().await;
        let err = svc
            .execute("get_secret", &json!({"key": "NOPE"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_set_secret_missing_key() {
        let svc = make_service().await;
        let err = svc
            .execute("set_secret", &json!({"value": "val"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_set_secret_missing_value() {
        let svc = make_service().await;
        let err = svc
            .execute("set_secret", &json!({"key": "k"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_list_secrets_after_set() {
        let svc = make_service().await;
        svc.execute("set_secret", &json!({"key": "K1", "value": "V1"}))
            .await
            .unwrap();
        svc.execute("set_secret", &json!({"key": "K2", "value": "V2"}))
            .await
            .unwrap();

        let result = svc.execute("list_secrets", &json!({})).await.unwrap();
        assert_eq!(result["secrets"].as_array().unwrap().len(), 2);
    }

    // ── Storage operations ──────────────────────────────────

    #[tokio::test]
    async fn test_write_and_read_storage() {
        let svc = make_service().await;
        let result = svc
            .execute(
                "write_storage",
                &json!({"key": "greeting", "value": "hello"}),
            )
            .await
            .unwrap();
        assert_eq!(result["stored"], true);

        let read = svc
            .execute("read_storage", &json!({"key": "greeting"}))
            .await
            .unwrap();
        assert_eq!(read["key"], "greeting");
        assert_eq!(read["value"], "hello");
    }

    #[tokio::test]
    async fn test_read_storage_all() {
        let svc = make_service().await;
        svc.execute("write_storage", &json!({"key": "a", "value": 1}))
            .await
            .unwrap();
        svc.execute("write_storage", &json!({"key": "b", "value": 2}))
            .await
            .unwrap();

        let result = svc.execute("read_storage", &json!({})).await.unwrap();
        assert!(result["storage"].is_object());
    }

    #[tokio::test]
    async fn test_write_storage_missing_key() {
        let svc = make_service().await;
        let err = svc
            .execute("write_storage", &json!({"value": "val"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_write_storage_missing_value() {
        let svc = make_service().await;
        let err = svc
            .execute("write_storage", &json!({"key": "k"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_write_storage_json_value() {
        let svc = make_service().await;
        svc.execute(
            "write_storage",
            &json!({"key": "obj", "value": {"nested": true}}),
        )
        .await
        .unwrap();

        let read = svc
            .execute("read_storage", &json!({"key": "obj"}))
            .await
            .unwrap();
        assert_eq!(read["value"]["nested"], true);
    }

    // ── Thread operations ───────────────────────────────────

    #[tokio::test]
    async fn test_list_threads_empty() {
        let svc = make_service().await;
        let result = svc.execute("list_threads", &json!({})).await.unwrap();
        assert!(result["threads"].is_array());
    }

    // ── Connection operations ───────────────────────────────

    #[tokio::test]
    async fn test_list_connections_no_store() {
        let svc = make_service().await;
        let err = svc
            .execute("list_connections", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not available"));
    }

    #[tokio::test]
    async fn test_get_connection_token_no_store() {
        let svc = make_service().await;
        let err = svc
            .execute("get_connection_token", &json!({"provider": "google"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not available"));
    }

    #[tokio::test]
    async fn test_get_connection_token_missing_provider() {
        let svc = make_service().await;
        let err = svc
            .execute("get_connection_token", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Missing"));
    }

    #[tokio::test]
    async fn test_list_connections_with_store() {
        let stores = initialize_stores(&test_store_config()).await.unwrap();
        let mock = MockConnectionStore::new()
            .with_connection("google", "connected")
            .with_connection("github", "disconnected");
        let svc = DistriPlatformService::new(
            stores,
            "user".to_string(),
            Some("ws-123".to_string()),
            Some(Arc::new(mock)),
        );
        let result = svc.execute("list_connections", &json!({})).await.unwrap();
        let conns = result["connections"].as_array().unwrap();
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[0]["name"], "google");
        assert_eq!(conns[1]["name"], "github");
    }

    #[tokio::test]
    async fn test_get_connection_token_with_store() {
        let stores = initialize_stores(&test_store_config()).await.unwrap();
        let mock = MockConnectionStore::new().with_token("google", "ya29.test-token");
        let svc = DistriPlatformService::new(
            stores,
            "user".to_string(),
            Some("ws-123".to_string()),
            Some(Arc::new(mock)),
        );
        let result = svc
            .execute("get_connection_token", &json!({"provider": "google"}))
            .await
            .unwrap();
        assert_eq!(result["access_token"], "ya29.test-token");
        assert_eq!(result["provider"], "google");
    }

    #[tokio::test]
    async fn test_get_connection_token_not_found() {
        let stores = initialize_stores(&test_store_config()).await.unwrap();
        let mock = MockConnectionStore::new();
        let svc = DistriPlatformService::new(
            stores,
            "user".to_string(),
            Some("ws-123".to_string()),
            Some(Arc::new(mock)),
        );
        let err = svc
            .execute("get_connection_token", &json!({"provider": "slack"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("slack"));
    }

    #[tokio::test]
    async fn test_connections_require_workspace_id() {
        let stores = initialize_stores(&test_store_config()).await.unwrap();
        let mock = MockConnectionStore::new();
        // No workspace_id
        let svc = DistriPlatformService::new(
            stores,
            "user".to_string(),
            None,
            Some(Arc::new(mock)),
        );
        let err = svc
            .execute("list_connections", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("workspace"));
    }

    // ── ACTIONS constant consistency ────────────────────────

    #[tokio::test]
    async fn test_every_action_in_registry_is_executable() {
        // Every action in ACTIONS should be handled in execute() (not return "Unknown action")
        let svc = make_service().await;
        for action_def in ACTIONS {
            let result = svc.execute(action_def.name, &json!({})).await;
            match result {
                Ok(_) => {} // Action succeeded (e.g. list_agents returns empty)
                Err(e) => {
                    let msg = e.to_string();
                    assert!(
                        !msg.contains("Unknown action"),
                        "Action '{}' is in ACTIONS but not handled in execute()",
                        action_def.name
                    );
                    // Other errors are fine (missing params, no store, etc.)
                }
            }
        }
    }
}
