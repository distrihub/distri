use std::sync::Arc;

use distri_parsers::get_available_tools;
use distri_types::{Action, AgentPlan, Part, PlanStep};

use crate::{
    agent::{strategy::planning::PlanningStrategy, ExecutorContext},
    types::Message,
    AgentError,
};

// True simple planner that doesn't do any planning

#[derive(Debug)]
pub struct SimplePlanner {
    pub agent_def: crate::types::StandardDefinition,
    pub strategy: crate::types::AgentStrategy,
}

impl SimplePlanner {
    pub fn new(
        agent_def: crate::types::StandardDefinition,
        strategy: crate::types::AgentStrategy,
    ) -> Self {
        Self {
            agent_def,
            strategy,
        }
    }

    async fn get_tool_instructions(&self, context: &Arc<ExecutorContext>) -> String {
        let tools = context.get_tools().await;
        get_available_tools(
            &tools
                .iter()
                .map(|t| t.get_tool_definition())
                .collect::<Vec<_>>(),
        )
    }

    /// Get planning template from prompt registry
    async fn get_planning_template(
        &self,
        context: &Arc<ExecutorContext>,
    ) -> Result<String, AgentError> {
        if let Some(orchestrator) = &context.orchestrator {
            if let Some(template) = orchestrator.get_prompt_template("planning").await {
                return Ok(template.content);
            }
        }
        Err(AgentError::Planning(
            "Planning template not found in prompt registry".to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl PlanningStrategy for SimplePlanner {
    async fn plan(
        &self,
        message: &Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        use handlebars::Handlebars;
        use serde_json::Value;
        use std::collections::HashMap;

        // Determine prompt to use
        let prompt = self.get_planning_template(&context).await?;

        // Create variables for template
        let mut variables = HashMap::new();

        let execution_history = context.get_execution_history().await;

        // Add context from previous conversation if available
        if !execution_history.is_empty() {
            let scratchpad = context.format_agent_scratchpad(Some(10)).await?;
            variables.insert("scratchpad".to_string(), Value::String(scratchpad));
        }

        // Add instructions if available

        if !self.agent_def.instructions.is_empty() {
            variables.insert(
                "instructions".to_string(),
                Value::String(self.agent_def.instructions.clone()),
            );
        }
        variables.insert(
            "task".to_string(),
            Value::String(message.as_text().unwrap_or_default()),
        );

        let tool_instructions = self.get_tool_instructions(&context).await;
        variables.insert(
            "tool_instructions".to_string(),
            Value::String(tool_instructions),
        );

        // Render template using handlebars
        let handlebars = Handlebars::new();
        let data = serde_json::to_value(variables)?;
        let prompt = handlebars
            .render_template(&prompt, &data)
            .map_err(|e| crate::AgentError::Other(format!("Template rendering error: {}", e)))?;

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

        tracing::debug!("Planning response: {:?}", response);

        let mut plan = AgentPlan::new(vec![]);
        if !response.tool_calls.is_empty() {
            plan.steps.push(PlanStep {
                id: uuid::Uuid::new_v4().to_string(),
                thought: Some(response.content.clone()),
                action: Action::ToolCalls {
                    tool_calls: response.tool_calls,
                },
            });
        }
        Ok(plan)
    }

    async fn plan_stream(
        &self,
        message: &Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        use handlebars::Handlebars;
        use serde_json::Value;
        use std::collections::HashMap;

        // Get tool descriptions with proper formatting
        let tool_instructions = self.get_tool_instructions(&context).await;

        // Determine prompt to use
        let prompt = self.get_planning_template(&context).await?;
        // Create variables for template
        let mut variables = HashMap::new();

        let history = context.get_execution_history().await;

        // Add context from previous conversation if available
        if !history.is_empty() {
            let scratchpad = context.format_agent_scratchpad(Some(10)).await?;
            variables.insert("scratchpad".to_string(), Value::String(scratchpad));
        }

        // Add instructions if available
        if !self.agent_def.instructions.is_empty() {
            variables.insert(
                "instructions".to_string(),
                Value::String(self.agent_def.instructions.clone()),
            );
        }
        variables.insert(
            "task".to_string(),
            Value::String(message.as_text().unwrap_or_default()),
        );

        variables.insert(
            "tool_instructions".to_string(),
            Value::String(tool_instructions),
        );

        // Render template using handlebars
        let handlebars = Handlebars::new();
        let data = serde_json::to_value(variables)?;
        let prompt = handlebars
            .render_template(&prompt, &data)
            .map_err(|e| crate::AgentError::Other(format!("Template rendering error: {}", e)))?;

        let mut plan_config = crate::types::PlanConfig::default();
        plan_config.model_settings = self.agent_def.model_settings.clone();

        let response = self
            .llm_stream(
                &[Message::system(prompt, None)],
                &plan_config,
                context.clone(),
                self.agent_def.tool_format.clone(),
            )
            .await?;

        tracing::debug!("Streaming planning response: {:?}", response);

        let mut plan = AgentPlan::new(vec![]);
        if !response.tool_calls.is_empty() {
            plan.steps.push(PlanStep {
                id: uuid::Uuid::new_v4().to_string(),
                thought: Some(response.content.clone()),
                action: Action::ToolCalls {
                    tool_calls: response.tool_calls,
                },
            });
        }
        Ok(plan)
    }
}
