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
            Some(false) => {
                // Use override_prompt if provided, otherwise default planning template
                self.agent_def.instructions.clone()
            }
            _ => {
                // Use agent instructions directly as handlebars template
                let mut instructions = self.agent_def.instructions.clone();
                let planning_template =
                    self.get_template_from_registry(context, "planning").await?;
                instructions.push_str(&format!("\n\n{}", planning_template));
                instructions
            }
        };

        // Call the private build_planning_prompt method
        self.build_messages(message, context, &template, &user_template)
            .await
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
                // Determine template based on prompt strategy
                let user_template = self.get_template_from_registry(&context, "user").await?;
                let template = match self.agent_def.append_default_instructions {
                    Some(false) => {
                        // Use override_prompt if provided, otherwise default planning template
                        self.agent_def.instructions.clone()
                    }
                    _ => {
                        // Use agent instructions directly as handlebars template
                        let mut instructions = self.agent_def.instructions.clone();
                        let planning_template = self
                            .get_template_from_registry(&context, "planning")
                            .await?;
                        instructions.push_str(&format!("\n\n{}", planning_template));
                        instructions
                    }
                };
                // Build planning prompt with agent instructions and context
                let mut messages = self
                    .build_messages(message, &context, &template, &user_template)
                    .await?;

                // include additional parts.

                // Get LLM response with retry logic for XML parsing failures
                let mut plan_config = crate::types::PlanConfig::default();
                plan_config.model_settings = self.agent_def.model_settings.clone();
                // Ensure we use the agent's effective context size, not the default
                plan_config.model_settings.context_size =
                    self.agent_def.get_effective_context_size();

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
                                } else {
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
                Ok(AgentPlan::new(
                    tool_calls
                        .into_iter()
                        .map(|tool_call| crate::types::PlanStep {
                            id: uuid::Uuid::new_v4().to_string(),
                            action: crate::types::Action::ToolCalls {
                                tool_calls: vec![tool_call],
                            },
                            thought: if thought.is_empty() {
                                None
                            } else {
                                Some(thought.clone())
                            },
                        })
                        .collect(),
                ))
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
    /// Build the complete planning prompt with context
    async fn build_messages(
        &self,
        message: &crate::types::Message,
        context: &Arc<ExecutorContext>,
        template: &str,
        user_template: &str,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
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
