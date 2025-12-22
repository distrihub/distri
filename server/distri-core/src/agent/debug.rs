use anyhow::Result;
use std::sync::Arc;

use crate::{
    agent::{strategy::planning::UnifiedPlanner, ExecutorContext, PlanningStrategy},
    llm::StreamResult,
    types::Message,
    AgentOrchestrator,
};

/// Generate the planning prompt for an agent without executing it
pub async fn generate_agent_prompt(
    executor: Arc<AgentOrchestrator>,
    agent_name: &str,
    message: &str,
    verbose: bool,
) -> Result<Vec<Message>> {
    let (context, agent_def) = get_debug_context_def(executor, agent_name, verbose).await?;

    // Create UnifiedPlanner with agent definition and strategy
    let strategy = agent_def.strategy.clone().unwrap_or_default();
    let planner = UnifiedPlanner::new(agent_def.clone(), strategy);

    // Create message from user input
    let user_message = Message::user(message.to_string(), None);

    // Generate the planning prompt using the planner's public method
    planner
        .generate_prompt_for_display(&user_message, &context)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to generate planning prompt: {}", e))
}

pub async fn get_debug_context_def(
    executor: Arc<AgentOrchestrator>,
    agent_name: &str,
    verbose: bool,
) -> Result<(Arc<ExecutorContext>, crate::types::StandardDefinition)> {
    // Get agent config from orchestrator
    let agent_config = executor
        .get_agent(agent_name)
        .await
        .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_name))?;

    // Convert agent config to StandardDefinition
    let agent_def = match agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(def) => def,
        _ => {
            return Err(anyhow::anyhow!(
                "Agent '{}' is not a standard agent - response generation not applicable",
                agent_name
            ));
        }
    };

    // Create a basic ExecutorContext for response generation
    let mut context = ExecutorContext::default();
    context.agent_id = agent_name.to_string();
    context.verbose = verbose;
    context.orchestrator = Some(executor.clone());
    context.stores = Some(executor.stores.clone());

    let context = Arc::new(context);

    Ok((context, agent_def))
}
/// Generate the LLM response for an agent without executing the agent loop
pub async fn generate_agent_response(
    executor: Arc<AgentOrchestrator>,
    agent_name: &str,
    message: &str,
    raw: bool,
    verbose: bool,
) -> Result<StreamResult> {
    // Create UnifiedPlanner with agent definition and strategy

    let (context, mut agent_def) = get_debug_context_def(executor, agent_name, verbose).await?;
    if raw {
        agent_def.tool_format = distri_types::ToolCallFormat::None;
    }
    let planner = UnifiedPlanner::new(
        agent_def.clone(),
        agent_def.strategy.clone().unwrap_or_default(),
    );

    // Create message from user input
    let user_message = Message::user(message.to_string(), None);

    // Generate the planning prompt using the planner's public method
    let messages = planner
        .generate_prompt_for_display(&user_message, &context)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to generate planning prompt: {}", e))?;

    // Create plan config for LLM streaming
    let mut plan_config = distri_types::PlanConfig::default();
    plan_config.model_settings = agent_def.model_settings.clone();

    // Use the planner's streaming method to get LLM response
    planner
        .llm_stream(
            &messages,
            &plan_config,
            context.clone(),
            agent_def.tool_format,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to generate LLM response: {}", e))
}
