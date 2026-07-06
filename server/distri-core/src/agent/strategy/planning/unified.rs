use std::sync::Arc;

use distri_stores::SessionStoreExt;
use distri_types::{AgentPlan, ExecutionResult, ExecutionStatus};

use crate::{agent::ExecutorContext, agent::PlanningStrategy, AgentError};

use super::formatter::MessageFormatter;

/// Unified planner that adapts behavior based on strategy configuration
#[derive(Debug)]
pub struct UnifiedPlanner {
    agent_def: crate::types::StandardDefinition,
    strategy: crate::types::AgentStrategy,
}

impl UnifiedPlanner {
    pub fn new(
        agent_def: crate::types::StandardDefinition,
        strategy: crate::types::AgentStrategy,
    ) -> Self {
        Self {
            agent_def,
            strategy,
        }
    }

    /// Group all tool calls from a single LLM response into ONE plan step.
    ///
    /// Previously each tool call became its own step, forcing one tool per
    /// agent-loop iteration with an LLM round-trip between every call — the
    /// model could request three things at once yet they ran strictly
    /// one-by-one. Returning a single step containing the whole batch lets the
    /// executor run them together (in parallel for concurrency-safe tools,
    /// serialized when any tool mutates shared state). An empty batch yields no
    /// steps.
    fn group_tool_calls_into_steps(
        tool_calls: Vec<crate::types::ToolCall>,
        thought: String,
    ) -> Vec<crate::types::PlanStep> {
        if tool_calls.is_empty() {
            return vec![];
        }
        vec![crate::types::PlanStep {
            id: uuid::Uuid::new_v4().to_string(),
            action: crate::types::Action::ToolCalls { tool_calls },
            thought: if thought.is_empty() {
                None
            } else {
                Some(thought)
            },
        }]
    }

    /// Strip XML content between ``` markers from the given content
    fn strip_xml_from_content(content: &str) -> String {
        use regex::Regex;

        // This regex matches a markdown code block with xml, e.g. ```xml ... ```
        let re = Regex::new(r"```xml\s*([\s\S]*?)\s*```").unwrap();

        // Replace XML blocks with empty string and clean up extra whitespace
        let stripped = re.replace_all(content, "");

        // Clean up multiple consecutive newlines
        let re_newlines = Regex::new(r"\n\s*\n\s*\n").unwrap();
        let cleaned = re_newlines.replace_all(&stripped, "\n\n");

        cleaned.trim().to_string()
    }
    /// Get a template from the prompt registry
    async fn get_template_from_registry(
        &self,
        context: &Arc<ExecutorContext>,
        template_name: &str,
    ) -> Result<String, AgentError> {
        if let Some(orchestrator) = &context.orchestrator {
            if let Some(template) = orchestrator.get_prompt_template(template_name).await {
                tracing::debug!("Using template '{}' from prompt registry", template_name);
                return Ok(template.content);
            }
        }

        Err(AgentError::Planning(format!(
            "Template '{}' not found in prompt registry and no orchestrator available",
            template_name
        )))
    }

    /// Generate the planning prompt for display/debugging purposes
    /// This is a public method specifically for CLI prompt generation
    pub async fn generate_prompt_for_display(
        &self,
        message: &crate::types::Message,
        context: &Arc<ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
        // Determine template based on prompt strategy (same logic as in plan method)
        let user_template = self.get_template_from_registry(&context, "user").await?;
        let template = match self.agent_def.append_default_instructions {
            Some(false) => self.agent_def.instructions.clone(),
            _ => {
                let mut instructions = self.agent_def.instructions.clone();
                let planning_template =
                    self.get_template_from_registry(context, "planning").await?;
                instructions.push_str(&format!("\n\n{}", planning_template));
                instructions
            }
        };

        // Call the private build_planning_prompt method (discard the budget for display purposes)
        let (messages, _budget) = self
            .build_messages(message, context, &template, &user_template)
            .await?;
        Ok(messages)
    }

    /// Shared function to format TODOs from context using session values
    pub async fn format_todos_from_context(
        context: &Arc<ExecutorContext>,
    ) -> Result<Option<String>, AgentError> {
        // Get todos from session using parent_task_id if available, otherwise task_id
        let task_id_for_key = context.parent_task_id.as_ref().unwrap_or(&context.task_id);
        let todos: Option<distri_types::TodoList> = context
            .get_session_store()?
            .get(task_id_for_key, "todos")
            .await
            .map_err(|e| AgentError::Session(format!("Failed to get todos from session: {}", e)))?;

        if let Some(todos) = todos {
            if !todos.items.is_empty() {
                Ok(Some(todos.format_display()))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

const MAX_RETRIES: usize = 2;

#[async_trait::async_trait]
impl PlanningStrategy for UnifiedPlanner {
    async fn build_summary_executor(
        &self,
        context: Arc<ExecutorContext>,
    ) -> Option<std::sync::Arc<dyn crate::llm::LLMExecutorTrait>> {
        // Reuse the agent's planning model for summarization but ship no
        // tools — the summarizer never calls them, and dragging the full
        // tool catalog into the summary prompt would defeat the purpose.
        let model_settings = self.agent_def.model_settings().cloned()?;
        let mut plan_config = crate::types::PlanConfig::default();
        plan_config.model_settings = Some(model_settings);
        let llm_def = crate::agent::strategy::planning::get_planning_definition(
            context.agent_id.clone(),
            plan_config.model_settings.clone(),
            crate::types::ToolCallFormat::default(),
        );
        crate::llm::create_llm_executor(llm_def, Vec::new(), context, None, None)
            .ok()
            .map(std::sync::Arc::from)
    }

    async fn plan(
        &self,
        message: &crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        // Use the appropriate planning behavior based on execution mode and configuration
        match self.strategy.get_execution_mode() {
            crate::types::ExecutionMode::Code { language } => {
                // Delegate to CodePlanner for pure code generation
                let code_planner = crate::agent::strategy::planning::CodePlanner::new(
                    language,
                    self.agent_def.clone(),
                    self.strategy.clone(),
                );
                code_planner.plan(message, context).await
            }
            crate::types::ExecutionMode::Tools => {
                let user_template = self.get_template_from_registry(&context, "user").await?;
                let template = match self.agent_def.append_default_instructions {
                    Some(false) => self.agent_def.instructions.clone(),
                    _ => {
                        let mut instructions = self.agent_def.instructions.clone();
                        let planning_template = self
                            .get_template_from_registry(&context, "planning")
                            .await?;
                        instructions.push_str(&format!("\n\n{}", planning_template));
                        instructions
                    }
                };
                // Build planning prompt with agent instructions and context
                let (mut messages, context_budget) = self
                    .build_messages(message, &context, &template, &user_template)
                    .await?;
                context.update_context_budget(context_budget).await;

                // include additional parts.

                // Get LLM response with retry logic for XML parsing failures
                let mut plan_config = crate::types::PlanConfig::default();
                plan_config.model_settings = self.agent_def.model_settings().cloned();
                // Ensure we use the agent's effective context size, not the default
                if let Some(ref mut ms) = plan_config.model_settings {
                    ms.inner.context_size = Some(self.agent_def.get_effective_context_size());
                }

                let response = {
                    let mut attempt = 0;
                    loop {
                        attempt += 1;

                        match self
                            .llm_stream(
                                &messages,
                                &plan_config,
                                context.clone(),
                                self.agent_def.tool_format.clone(),
                            )
                            .await
                        {
                            Ok(response) => {
                                if !response.tool_calls.is_empty() {
                                    break Ok(response);
                                } else if attempt < MAX_RETRIES {
                                    let err = "You always need to return tool calls in the response, but got:";
                                    messages.push(crate::types::Message::assistant(
                                        response.content.clone(),
                                        None,
                                    ));
                                    messages.push(crate::types::Message::user(
                                        format!(
                                            "(attempt {}/{}): {}. Retrying...",
                                            attempt, MAX_RETRIES, err
                                        ),
                                        None,
                                    ));
                                    continue;
                                } else {
                                    // LLM refused to produce a tool call after
                                    // MAX_RETRIES. Treat the final text content
                                    // as the answer and auto-finalize so the
                                    // agent loop doesn't spin forever. Common
                                    // trigger: an ad-hoc sub-agent whose
                                    // `system_prompt` tells the model to emit
                                    // text-only output (e.g. "return only valid
                                    // MDX") — the model obeys, never calls
                                    // `final`, and every outer iteration re-
                                    // invokes planning which repeats the same
                                    // text. Fall back to the text as the final
                                    // result.
                                    tracing::warn!(
                                        agent = %self.agent_def.name,
                                        attempts = attempt,
                                        "Planner: LLM returned text-only after {} retries; auto-finalizing",
                                        MAX_RETRIES
                                    );
                                    context
                                        .set_final_result(Some(serde_json::Value::String(
                                            response.content.clone(),
                                        )))
                                        .await;
                                    break Ok(response);
                                }
                            }
                            Err(AgentError::XmlParsingFailed(ref content, ref err))
                                if attempt < MAX_RETRIES =>
                            {
                                tracing::warn!(
                                    "XML parsing failed during planning (attempt {}/{}): {}. Retrying...",
                                    attempt,
                                    MAX_RETRIES,
                                    err
                                );

                                messages
                                    .push(crate::types::Message::assistant(content.clone(), None));
                                messages.push(crate::types::Message::user(
                                    format!("XML parsing failed during planning (attempt {}/{}): {}. Retrying...", 
                                    attempt, MAX_RETRIES, err),
                                    None,
                                ));
                                continue;
                            }
                            Err(e) => {
                                break Err(e);
                            }
                        }
                    }
                }?;

                // Parse response into plan steps using simple parser
                let tool_calls = response.tool_calls;

                let thought = Self::strip_xml_from_content(&response.content);
                Ok(AgentPlan::new(Self::group_tool_calls_into_steps(
                    tool_calls, thought,
                )))
            }
        }
    }

    async fn replan(
        &self,
        message: &crate::types::Message,
        context: Arc<ExecutorContext>,
        previous_plan: AgentPlan,
    ) -> Result<AgentPlan, AgentError> {
        // Check if replanning is enabled
        let replanning_config = self.strategy.get_replanning();
        if !replanning_config.is_enabled() {
            return Ok(previous_plan);
        }

        // Delegate to plan method for replanning
        self.plan(message, context).await
    }

    fn needs_replanning(&self, execution_history: &[ExecutionResult]) -> bool {
        let replanning_config = self.strategy.get_replanning();
        if !replanning_config.is_enabled() {
            return false;
        }

        match replanning_config.get_trigger() {
            crate::types::ReplanningTrigger::Never => false,
            crate::types::ReplanningTrigger::AfterFailures => {
                // Check if there are recent failures
                execution_history
                    .iter()
                    .rev()
                    .take(3)
                    .any(|result| result.status == ExecutionStatus::Failed)
            }
            crate::types::ReplanningTrigger::AfterNIterations(n) => execution_history.len() >= n,
            crate::types::ReplanningTrigger::AfterReflection => {
                // This would be triggered by the reflection system
                false
            }
        }
    }
}

impl UnifiedPlanner {
    /// Build the complete planning prompt with context.
    /// Returns the message list and a ContextBudget with per-component token estimates.
    async fn build_messages(
        &self,
        message: &crate::types::Message,
        context: &Arc<ExecutorContext>,
        template: &str,
        user_template: &str,
    ) -> Result<(Vec<crate::types::Message>, distri_types::ContextBudget), AgentError> {
        let todos = if self.agent_def.is_todos_enabled() {
            Self::format_todos_from_context(&context).await?
        } else {
            None
        };

        let formatter = MessageFormatter::new(&self.agent_def, &self.strategy);
        formatter
            .build_messages(message, context, template, user_template, todos)
            .await
    }
}

#[cfg(test)]
mod grouping_tests {
    use super::*;
    use crate::types::{Action, ToolCall};
    use serde_json::json;

    fn tc(id: &str, name: &str) -> ToolCall {
        ToolCall {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            input: json!({}),
        }
    }

    #[test]
    fn multiple_tool_calls_collapse_into_one_step() {
        let calls = vec![tc("a", "search"), tc("b", "browsr_scrape"), tc("c", "search")];
        let steps = UnifiedPlanner::group_tool_calls_into_steps(calls, "thinking".to_string());

        // The whole batch must land in a SINGLE step — this is the fix for
        // one-tool-per-iteration sequential execution.
        assert_eq!(steps.len(), 1, "expected one grouped step, got {}", steps.len());
        match &steps[0].action {
            Action::ToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 3, "all three calls must be grouped");
                assert_eq!(tool_calls[0].tool_call_id, "a");
                assert_eq!(tool_calls[1].tool_call_id, "b");
                assert_eq!(tool_calls[2].tool_call_id, "c");
            }
            other => panic!("expected Action::ToolCalls, got {other:?}"),
        }
        assert_eq!(steps[0].thought.as_deref(), Some("thinking"));
    }

    #[test]
    fn single_tool_call_is_one_step() {
        let steps =
            UnifiedPlanner::group_tool_calls_into_steps(vec![tc("only", "final")], String::new());
        assert_eq!(steps.len(), 1);
        match &steps[0].action {
            Action::ToolCalls { tool_calls } => assert_eq!(tool_calls.len(), 1),
            other => panic!("expected Action::ToolCalls, got {other:?}"),
        }
        // Empty thought must not produce a Some("").
        assert_eq!(steps[0].thought, None);
    }

    #[test]
    fn no_tool_calls_yields_no_steps() {
        let steps = UnifiedPlanner::group_tool_calls_into_steps(vec![], "x".to_string());
        assert!(steps.is_empty());
    }
}
