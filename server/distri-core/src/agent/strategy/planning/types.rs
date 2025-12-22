use distri_types::{AgentPlan, ExecutionResult, ToolCallFormat};
use serde::{Deserialize, Serialize};
use std::{fmt::Display, sync::Arc};

use crate::{
    agent::{strategy::planning::get_planning_definition, ExecutorContext},
    llm::{LLMResponse, StreamResult},
    AgentError,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "value")]
pub enum Step {
    Plan(String),
    Thought(String),
    Action(String),
    Observation(String),
}

impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Plan(plan) => write!(f, "Plan: {}", plan),
            Step::Thought(thought) => write!(f, "Thought: {}", thought),
            Step::Action(action) => write!(f, "Action: {}", action),
            Step::Observation(observation) => write!(f, "Observation: {}", observation),
        }
    }
}

/// Strategy for planning agent actions
#[async_trait::async_trait]
pub trait PlanningStrategy: Send + Sync + std::fmt::Debug {
    async fn plan(
        &self,
        message: &crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError>;

    /// Streaming version of plan that emits tokens as they're generated for better UX
    async fn plan_stream(
        &self,
        message: &crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        // Default implementation falls back to non-streaming
        self.plan(message, context).await
    }

    async fn replan(
        &self,
        message: &crate::types::Message,
        context: Arc<ExecutorContext>,
        _previous_plan: AgentPlan,
    ) -> Result<AgentPlan, AgentError> {
        self.plan(message, context).await
    }

    // By default we dont replan
    fn needs_replanning(&self, _history: &[ExecutionResult]) -> bool {
        false
    }

    async fn get_tool_descriptions(&self, context: &Arc<ExecutorContext>) -> String {
        use distri_parsers::get_tool_descriptions;
        get_tool_descriptions(
            &context
                .get_tools()
                .await
                .iter()
                .map(|t| t.get_tool_definition())
                .collect::<Vec<_>>(),
        )
    }

    async fn llm(
        &self,
        messages: &[crate::types::Message],
        plan_config: &crate::types::PlanConfig,
        context: Arc<ExecutorContext>,
        format: ToolCallFormat,
    ) -> Result<LLMResponse, AgentError> {
        let agent_name = context.agent_id.clone();
        let tools = context.get_tools().await;
        let planning_executor = crate::llm::LLMExecutor::new(
            get_planning_definition(agent_name, plan_config.model_settings.clone(), format),
            tools,
            context.clone(),
            None,
            None,
        );

        planning_executor.execute(&messages).await
    }

    /// Streaming version of llm helper method
    async fn llm_stream(
        &self,
        messages: &[crate::types::Message],
        plan_config: &crate::types::PlanConfig,
        context: Arc<ExecutorContext>,
        format: ToolCallFormat,
    ) -> Result<StreamResult, AgentError> {
        let agent_name = context.agent_id.clone();
        let tools = context.get_tools().await;
        let planning_executor = crate::llm::LLMExecutor::new(
            get_planning_definition(agent_name, plan_config.model_settings.clone(), format),
            tools,
            context.clone(),
            None,
            None,
        );

        planning_executor
            .execute_stream(&messages, context.clone())
            .await
    }
}
