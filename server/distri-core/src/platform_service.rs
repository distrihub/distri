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
];

pub struct ActionDef {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
}

pub struct DistriPlatformService {
    stores: InitializedStores,
    user_id: String,
}

impl DistriPlatformService {
    pub fn new(stores: InitializedStores, user_id: String) -> Self {
        Self { stores, user_id }
    }

    /// Execute a platform action by name with JSON params.
    /// Returns a JSON result.
    pub async fn execute(&self, action: &str, params: &Value) -> Result<Value> {
        match action {
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

        let service =
            DistriPlatformService::new(orchestrator.stores.clone(), context.user_id.clone());

        match service.execute(action, &params).await {
            Ok(result) => Ok(vec![Part::Data(result)]),
            Err(e) => Err(AgentError::ToolExecution(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_actions() {
        let actions = DistriPlatformService::list_actions();
        assert!(actions["actions"].is_array());
        let arr = actions["actions"].as_array().unwrap();
        assert!(arr.len() > 10);
        // Check known actions exist
        assert!(arr.iter().any(|a| a["name"] == "list_agents"));
        assert!(arr.iter().any(|a| a["name"] == "create_skill"));
        assert!(arr.iter().any(|a| a["name"] == "set_secret"));
        assert!(arr.iter().any(|a| a["name"] == "list_threads"));
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
}
