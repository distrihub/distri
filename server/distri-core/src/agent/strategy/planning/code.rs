use std::sync::Arc;

use distri_types::{Action, AgentPlan, Part, PlanStep};

use crate::{
    agent::{strategy::planning::PlanningStrategy, types::MAX_ITERATIONS, ExecutorContext},
    types::{CodeLanguage, Message},
    AgentError,
};

/// Planner that generates executable code steps using code.hbs template
#[derive(Debug)]
pub struct CodePlanner {
    pub language: CodeLanguage,
    pub agent_def: crate::types::StandardDefinition,
    pub strategy: crate::types::AgentStrategy,
}

pub struct CodePlan {
    pub code: String,
    pub thought: Option<String>,
}

impl CodePlanner {
    pub fn new(
        language: CodeLanguage,
        agent_def: crate::types::StandardDefinition,
        strategy: crate::types::AgentStrategy,
    ) -> Self {
        Self {
            language,
            agent_def,
            strategy,
        }
    }

    /// Static method to parse TypeScript code from content
    pub fn parse_typescript_from_content(content: &str) -> Option<CodePlan> {
        let patterns = vec!["```typescript\n", "```ts\n", "```javascript\n", "```js\n"];

        let mut codes = Vec::new();
        let mut current_pos = 0;

        while current_pos < content.len() {
            let mut found_start = None;
            let mut found_pattern = "";

            // Find the earliest occurrence of any pattern
            for pattern in &patterns {
                if let Some(start) = content[current_pos..].find(pattern) {
                    let abs_start = current_pos + start;
                    if found_start.is_none() || abs_start < found_start.unwrap() {
                        found_start = Some(abs_start);
                        found_pattern = pattern;
                    }
                }
            }

            if let Some(start_pos) = found_start {
                let code_start = start_pos + found_pattern.len();
                if let Some(end) = content[code_start..].find("```") {
                    let code = content[code_start..code_start + end].trim();
                    if !code.is_empty() {
                        codes.push(code.to_string());
                    }
                    current_pos = code_start + end + 3; // Skip "```"
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if codes.is_empty() {
            None
        } else {
            // Extract thoughts from content (simplified) - now looks for plain text, not XML tags
            let thought = Self::extract_thoughts_from_content(content);

            Some(CodePlan {
                code: codes.join("\n\n"),
                thought,
            })
        }
    }

    pub fn extract_thoughts_from_content(content: &str) -> Option<String> {
        let mut thoughts = Vec::new();
        let content_lines: Vec<&str> = content.lines().collect();
        let mut in_code_block = false;

        for line in content_lines {
            if line.trim().starts_with("```typescript")
                || line.trim().starts_with("```ts")
                || line.trim().starts_with("```javascript")
                || line.trim().starts_with("```js")
            {
                in_code_block = true;
                continue;
            }
            if line.trim() == "```" && in_code_block {
                in_code_block = false;
                continue;
            }

            // If we're not in a code block and the line has content, it's a thought
            if !in_code_block && !line.trim().is_empty() {
                let trimmed = line.trim();
                if !trimmed.starts_with('#')
                    && !trimmed.starts_with('*')
                    && !trimmed.starts_with('-')
                {
                    thoughts.push(trimmed);
                }
            }
        }

        if thoughts.is_empty() {
            None
        } else {
            Some(thoughts.join(" "))
        }
    }
}

#[async_trait::async_trait]
impl PlanningStrategy for CodePlanner {
    async fn plan(
        &self,
        message: &Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        use handlebars::Handlebars;
        use serde_json::Value;
        use std::collections::HashMap;

        // Get tool descriptions with XML formatting
        let tools = self.get_tool_descriptions(&context).await;

        let code_template = include_str!("./templates/code.hbs");

        // Create variables for template
        let mut variables = HashMap::new();
        variables.insert("tools".to_string(), Value::String(tools));

        // Add the user task - this is the most important part!
        variables.insert(
            "task".to_string(),
            Value::String(message.as_text().unwrap_or_default()),
        );

        let scratchpad = context.format_agent_scratchpad(Some(10)).await?;
        variables.insert("scratchpad".to_string(), Value::String(scratchpad));

        // Add agent instructions if available
        if !self.agent_def.instructions.is_empty() {
            variables.insert(
                "instructions".to_string(),
                Value::String(self.agent_def.instructions.clone()),
            );
        }

        // Add configuration values for response depth guidance
        if let Some(strategy) = &self.agent_def.strategy {
            let reasoning_depth = strategy.get_reasoning_depth();

            // Calculate remaining steps

            variables.insert(
                "reasoning_depth".to_string(),
                Value::String(match reasoning_depth {
                    crate::types::ReasoningDepth::Shallow => "shallow".to_string(),
                    crate::types::ReasoningDepth::Standard => "standard".to_string(),
                    crate::types::ReasoningDepth::Deep => "deep".to_string(),
                }),
            );
        }

        let usage_steps = context.get_usage().await.current_iteration;
        let max_steps = self.agent_def.max_iterations.unwrap_or(MAX_ITERATIONS);
        // Rely on tracked iteration count instead of execution history length (history may be disabled)
        let current_steps = usage_steps;
        let remaining_steps = max_steps.saturating_sub(current_steps);
        variables.insert(
            "max_steps".to_string(),
            Value::Number(serde_json::Number::from(max_steps)),
        );
        variables.insert(
            "current_steps".to_string(),
            Value::Number(serde_json::Number::from(current_steps)),
        );
        variables.insert(
            "remaining_steps".to_string(),
            Value::Number(serde_json::Number::from(remaining_steps)),
        );

        // Get current TODOs from session if enabled
        if self.agent_def.is_todos_enabled() {
            if let Some(todos) =
                super::unified::UnifiedPlanner::format_todos_from_context(&context).await?
            {
                variables.insert("todos".to_string(), Value::String(todos));
            }
        }

        // Render template using handlebars
        let mut handlebars = Handlebars::new();

        // Register TODO partials for code template
        handlebars
            .register_partial(
                "todo_instructions",
                include_str!(
                    "../../../../../../distri-types/prompt_templates/partials/todo_instructions.hbs"
                ),
            )
            .unwrap();
        let data = serde_json::to_value(variables)
            .map_err(|e| crate::AgentError::Other(format!("JSON serialization error: {}", e)))?;

        // Debug: log variables being passed to template
        tracing::debug!("[CodePlanner] Template variables: {:?}", data);

        let prompt = handlebars
            .render_template(&code_template, &data)
            .map_err(|e| {
                tracing::error!("[CodePlanner] Template rendering failed: {}", e);
                tracing::error!("[CodePlanner] Template: {}", code_template);
                tracing::error!("[CodePlanner] Variables: {:?}", data);
                crate::AgentError::Other(format!("Template rendering error: {}", e))
            })?;
        let mut plan_config = crate::types::PlanConfig::default();
        plan_config.model_settings = self.agent_def.model_settings.clone();

        let mut messages = vec![Message::system(prompt, None)];
        // Only include additional user message if has images
        let mut message = message.clone();
        message.parts.retain(|p| matches!(p, Part::Image(_)));
        if message.parts.len() > 0 {
            messages.push(message);
        }

        let response = self
            .llm_stream(
                &messages,
                &plan_config,
                context.clone(),
                self.agent_def.tool_format.clone(),
            )
            .await?;

        let code_plan = CodePlanner::parse_typescript_from_content(&response.content);
        if let Some(code_plan) = code_plan {
            let mut plan = AgentPlan::new(vec![]);
            plan.steps.push(PlanStep {
                id: uuid::Uuid::new_v4().to_string(),
                thought: code_plan.thought,
                action: Action::Code {
                    code: code_plan.code,
                    language: self.language.to_string(),
                },
            });
            Ok(plan)
        } else {
            let content = format!(
                "ERROR: Failed to get code from response: {}",
                response.content
            );
            return Err(AgentError::Planning(content));
        }
    }
}
