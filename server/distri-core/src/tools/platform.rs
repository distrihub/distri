//! Platform management tools for system agents.
//! These tools allow the gateway agent to manage agents, skills, and storage.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};

/// Names of all platform tools, used for auto-inclusion in system agents.
pub const PLATFORM_TOOL_NAMES: &[&str] = &[
    "list_agents",
    "list_skills",
    "create_skill",
    "delete_skill",
    "write_to_storage",
    "read_from_storage",
];

/// Returns all platform management tools as Arc<dyn Tool>.
pub fn get_platform_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ListAgentsTool) as Arc<dyn Tool>,
        Arc::new(ListSkillsTool) as Arc<dyn Tool>,
        Arc::new(CreateSkillTool) as Arc<dyn Tool>,
        Arc::new(DeleteSkillTool) as Arc<dyn Tool>,
        Arc::new(WriteToStorageTool) as Arc<dyn Tool>,
        Arc::new(ReadFromStorageTool) as Arc<dyn Tool>,
    ]
}

// ── list_agents ─────────────────────────────────────────────

#[derive(Debug)]
pub struct ListAgentsTool;

#[async_trait::async_trait]
impl Tool for ListAgentsTool {
    fn get_name(&self) -> String {
        "list_agents".to_string()
    }
    fn get_description(&self) -> String {
        "List all agents in the current workspace".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ListAgentsTool {
    async fn execute_with_executor_context(
        &self,
        _tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let (agents, _cursor) = orchestrator.stores.agent_store.list(None, Some(100)).await;
        let agent_summaries: Vec<Value> = agents
            .iter()
            .map(|config| {
                let def = config.get_definition();
                json!({
                    "name": def.name,
                    "description": def.description,
                    "model": &def.model_settings.model,
                })
            })
            .collect();
        Ok(vec![Part::Data(json!({ "agents": agent_summaries }))])
    }
}

// ── list_skills ─────────────────────────────────────────────

#[derive(Debug)]
pub struct ListSkillsTool;

#[async_trait::async_trait]
impl Tool for ListSkillsTool {
    fn get_name(&self) -> String {
        "list_skills".to_string()
    }
    fn get_description(&self) -> String {
        "List available skills in the workspace".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ListSkillsTool {
    async fn execute_with_executor_context(
        &self,
        _tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| AgentError::ToolExecution("Skill store not available".to_string()))?;
        let skills = skill_store
            .list_skills()
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
        let skill_summaries: Vec<Value> = skills
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
        Ok(vec![Part::Data(json!({ "skills": skill_summaries }))])
    }
}

// ── create_skill ────────────────────────────────────────────

#[derive(Debug)]
pub struct CreateSkillTool;

#[async_trait::async_trait]
impl Tool for CreateSkillTool {
    fn get_name(&self) -> String {
        "create_skill".to_string()
    }
    fn get_description(&self) -> String {
        "Create a new reusable skill".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill name" },
                "description": { "type": "string", "description": "Short description" },
                "content": { "type": "string", "description": "Skill content (markdown)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization" }
            },
            "required": ["name", "content"]
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for CreateSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| AgentError::ToolExecution("Skill store not available".to_string()))?;

        let input = &tool_call.input;
        let name = input["name"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecution("Missing 'name'".to_string()))?;
        let content = input["content"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecution("Missing 'content'".to_string()))?;
        let description = input["description"].as_str().map(|s| s.to_string());
        let tags: Vec<String> = input["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let new_skill = distri_types::stores::NewSkill {
            name: name.to_string(),
            description,
            content: content.to_string(),
            tags,
            is_public: false,
            scripts: vec![],
        };
        let record = skill_store
            .create_skill(new_skill)
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
        Ok(vec![Part::Data(json!({
            "id": record.id,
            "name": record.name,
            "created": true
        }))])
    }
}

// ── delete_skill ────────────────────────────────────────────

#[derive(Debug)]
pub struct DeleteSkillTool;

#[async_trait::async_trait]
impl Tool for DeleteSkillTool {
    fn get_name(&self) -> String {
        "delete_skill".to_string()
    }
    fn get_description(&self) -> String {
        "Delete a skill by ID. Always confirm with the user first.".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string", "description": "The skill ID to delete" }
            },
            "required": ["skill_id"]
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for DeleteSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| AgentError::ToolExecution("Skill store not available".to_string()))?;

        let skill_id = tool_call.input["skill_id"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecution("Missing 'skill_id'".to_string()))?;

        skill_store
            .delete_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
        Ok(vec![Part::Data(json!({ "deleted": true, "skill_id": skill_id }))])
    }
}

// ── write_to_storage ────────────────────────────────────────

#[derive(Debug)]
pub struct WriteToStorageTool;

#[async_trait::async_trait]
impl Tool for WriteToStorageTool {
    fn get_name(&self) -> String {
        "write_to_storage".to_string()
    }
    fn get_description(&self) -> String {
        "Store information persistently (survives thread resets)".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Storage key" },
                "value": { "description": "Value to store (any JSON type)" }
            },
            "required": ["key", "value"]
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for WriteToStorageTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let key = tool_call.input["key"]
            .as_str()
            .ok_or_else(|| AgentError::ToolExecution("Missing 'key'".to_string()))?;
        let value = &tool_call.input["value"];
        if value.is_null() {
            return Err(AgentError::ToolExecution("Missing 'value'".to_string()));
        }

        // Use a per-user namespace: "platform_storage:{user_id}"
        let namespace = format!("platform_storage:{}", context.user_id);
        orchestrator
            .stores
            .session_store
            .set_value(&namespace, key, value)
            .await
            .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
        Ok(vec![Part::Data(json!({ "stored": true, "key": key }))])
    }
}

// ── read_from_storage ───────────────────────────────────────

#[derive(Debug)]
pub struct ReadFromStorageTool;

#[async_trait::async_trait]
impl Tool for ReadFromStorageTool {
    fn get_name(&self) -> String {
        "read_from_storage".to_string()
    }
    fn get_description(&self) -> String {
        "Read stored information. Omit key to list everything.".to_string()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }
    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Storage key to read (omit to list all)" }
            }
        })
    }
    async fn execute(&self, _: ToolCall, _: Arc<ToolContext>) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("Requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ReadFromStorageTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let orchestrator = context.get_orchestrator()?;
        let namespace = format!("platform_storage:{}", context.user_id);

        if let Some(key) = tool_call.input["key"].as_str() {
            let value = orchestrator
                .stores
                .session_store
                .get_value(&namespace, key)
                .await
                .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
            Ok(vec![Part::Data(json!({ "key": key, "value": value }))])
        } else {
            let all = orchestrator
                .stores
                .session_store
                .get_all_values(&namespace)
                .await
                .map_err(|e| AgentError::ToolExecution(e.to_string()))?;
            Ok(vec![Part::Data(json!({ "storage": all }))])
        }
    }
}
