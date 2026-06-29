use std::sync::Arc;

use distri_types::{stores::ContextExecutionType, tool::ToolContext, Part, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
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
            .get(skill_id)
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

                // Render the skill body via the SAME pipeline the
                // formatter uses for the agent's own system prompt
                // (`render_prompt` resolves `{{> partial}}` references
                // against the registry, then renders). Reusing it
                // means `{{runtime_mode}}` / `{{#if (eq runtime_mode
                // "cli")}}…{{/if}}` resolve identically regardless of
                // who emitted the template text. Render failure (almost
                // always a typo / unbalanced `{{` in the skill body)
                // bubbles up as a `ToolExecution` error so the skill
                // author sees the real problem instead of an LLM run
                // that silently misbehaves around an unrendered template.
                let template_data = distri_types::prompt::TemplateData {
                    runtime_mode: context.runtime_mode.as_template_name(),
                    ..Default::default()
                };
                let rendered_body = crate::agent::strategy::planning::formatter::render_prompt(
                    &context,
                    &skill.content,
                    &template_data,
                )
                .await
                .map_err(|e| {
                    AgentError::ToolExecution(format!(
                        "load_skill('{skill_id}'): template render failed: {e}. \
                         Check the skill body for unbalanced `{{{{` / `}}}}` or \
                         references to a partial that isn't registered."
                    ))
                })?;

                // Return full content with no truncation. The agent incorporates it
                // directly into the current conversation turn. If the skill specifies
                // a model override, surface it as a note so the agent is aware.
                let content = if let Some(ref model) = skill.model {
                    format!(
                        "{}\n\n<!-- skill preferred model: {} -->",
                        rendered_body, model
                    )
                } else {
                    rendered_body
                };
                // Track skill for post-compaction re-injection
                {
                    let mut tracker = context.skill_tracker.write().await;
                    tracker.track(
                        skill_id.to_string(),
                        content.clone(),
                        chrono::Utc::now().timestamp_millis(),
                    );
                }
                Ok(vec![Part::Text(content)])
            }

            ContextExecutionType::Fork => {
                // Single source of truth for fork semantics (also used by the
                // metadata-driven `preload_skills` path): same thread, fresh
                // task_id/run_id, parent_task_id = current task, skill body as
                // the child's instructions; only the gist returns to the parent.
                let summary = context
                    .fork_skill(skill_id, &skill.content, skill.model.clone())
                    .await
                    .unwrap_or_else(|e| {
                        tracing::error!(skill_id = skill_id, error = %e, "Forked skill execution failed");
                        format!("[Skill '{skill_id}' result]\nSkill '{skill_id}' failed: {e}")
                    });

                Ok(vec![Part::Text(summary)])
            }
        }
    }
}
