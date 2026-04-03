use std::sync::Arc;

use distri_types::{
    configuration::DefinitionOverrides, stores::ContextExecutionType, tool::ToolContext,
    MessageRole, Part, ToolCall,
};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::Message;
use crate::AgentError;

/// Tool that loads a skill's content on demand.
///
/// **Inline context (default):** Returns the full skill markdown as text directly
/// into the calling agent's conversation. No sub-agent is spawned; the calling
/// agent incorporates the content in the current turn.
///
/// **Fork context:** Spawns an isolated child agent via `context.new_task()`.
/// The child runs with the skill content as its instruction set and its own
/// token budget. The parent receives a summary of the child's result. The
/// parent–child relationship is persisted in the task store (`parent_task_id`).
#[derive(Debug, Clone)]
pub struct LoadSkillTool;

#[async_trait::async_trait]
impl distri_types::Tool for LoadSkillTool {
    fn get_name(&self) -> String {
        "load_skill".to_string()
    }

    fn get_description(&self) -> String {
        "Load a skill by its ID. Returns the skill's full content including instructions and available scripts.".to_string()
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

        match skill.context {
            ContextExecutionType::Inline => {
                tracing::info!(
                    skill_id = skill_id,
                    content_len = skill.content.len(),
                    "Loading skill inline — injecting into current context"
                );
                // Return full content with no truncation. The agent incorporates it
                // directly into the current conversation turn. If the skill specifies
                // a model override, surface it as a note so the agent is aware.
                let content = if let Some(ref model) = skill.model {
                    format!(
                        "{}\n\n<!-- skill preferred model: {} -->",
                        skill.content, model
                    )
                } else {
                    skill.content.clone()
                };
                Ok(vec![Part::Text(content)])
            }

            ContextExecutionType::Fork => {
                tracing::info!(
                    skill_id = skill_id,
                    model = skill.model.as_deref().unwrap_or("(agent default)"),
                    "Forking skill — spawning isolated child agent"
                );

                // Fork the execution context. This:
                //   - Keeps the same thread_id (child is part of same conversation)
                //   - Generates a new task_id + run_id (isolated work unit)
                //   - Sets parent_task_id = current task_id (hierarchy in task store)
                //   - Gives the child a fresh ContextUsage counter (isolated budget)
                //
                // orchestrator.execute() will then persist via:
                //   task_store.get_or_create_task(thread_id, task_id)
                //   task_store.update_parent_task(task_id, parent_task_id)
                let child_context = context.new_task(&context.agent_id).await;

                // Build definition overrides: inject skill content as instructions,
                // optionally override the model if the skill specifies one.
                let overrides = {
                    let base =
                        DefinitionOverrides::default().with_instructions(skill.content.clone());
                    if let Some(model) = skill.model.clone() {
                        base.with_model(model)
                    } else {
                        base
                    }
                };

                let message = Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: None,
                    parts: vec![Part::Text(format!(
                        "Execute the skill '{}' according to your instructions.",
                        skill_id
                    ))],
                    role: MessageRole::User,
                    created_at: chrono::Utc::now().timestamp_millis(),
                    agent_id: None,
                    parts_metadata: None,
                };

                let child_context_arc = Arc::new(child_context);
                let child_context_for_result = child_context_arc.clone();

                let invoke_result = orchestrator
                    .execute_stream(
                        &context.agent_id,
                        message,
                        child_context_arc,
                        Some(overrides),
                    )
                    .await;

                let summary = match invoke_result {
                    Ok(result) => {
                        let final_result = child_context_for_result.get_final_result().await;
                        final_result
                            .and_then(|v| match v {
                                serde_json::Value::String(s) => Some(s),
                                other => Some(other.to_string()),
                            })
                            .or(result.content)
                            .unwrap_or_else(|| {
                                format!("Skill '{}' completed without output.", skill_id)
                            })
                    }
                    Err(e) => {
                        tracing::error!(skill_id = skill_id, error = %e, "Forked skill execution failed");
                        format!("Skill '{}' failed: {}", skill_id, e)
                    }
                };

                Ok(vec![Part::Text(format!(
                    "[Skill '{}' result]\n{}",
                    skill_id, summary
                ))])
            }
        }
    }
}
