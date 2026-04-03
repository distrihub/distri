use std::sync::Arc;

use distri_types::{tool::ToolContext, Part, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::AgentError;

/// Tool that loads a skill's content on demand.
/// The agent calls this tool when it needs a specific skill.
/// The skill's markdown content is returned as-is, including
/// any instructions and tool usage details embedded within.
#[derive(Debug, Clone)]
pub struct LoadSkillTool;

#[async_trait::async_trait]
impl distri_types::Tool for LoadSkillTool {
    fn get_name(&self) -> String {
        "load_skill".to_string()
    }

    fn get_description(&self) -> String {
        "Load a skill by its ID. Returns the skill's full content including instructions, tool usage documentation, and available scripts.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The ID of the skill to load"
                }
            },
            "required": ["skill_id"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "LoadSkillTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for LoadSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let skill_id = tool_call
            .input
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: skill_id".to_string())
            })?;

        let orchestrator = context.get_orchestrator()?;
        let skill_store =
            orchestrator.stores.skill_store.as_ref().ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| AgentError::ToolExecution(format!("Skill '{}' not found", skill_id)))?;

        // Respect max_tokens budget: truncate content if it exceeds the per-skill cap.
        // Default cap is 5000 tokens (~20K chars). This prevents a single skill from
        // consuming too much of the context window.
        let max_tokens = skill
            .max_tokens
            .unwrap_or(distri_types::stores::DEFAULT_SKILL_MAX_TOKENS);
        let max_chars = max_tokens * 4; // ~4 chars per token heuristic
        let content = if skill.content.len() > max_chars {
            let truncated: String = skill.content.chars().take(max_chars).collect();
            format!(
                "{}\n\n[... truncated at {} token budget ({} chars of {} total)]",
                truncated,
                max_tokens,
                max_chars,
                skill.content.len()
            )
        } else {
            skill.content.clone()
        };

        let estimated_tokens = (content.len() + 3) / 4;
        tracing::info!(
            "Loaded skill '{}' ({} tokens, budget: {})",
            skill_id,
            estimated_tokens,
            max_tokens
        );

        Ok(vec![Part::Text(content)])
    }
}
