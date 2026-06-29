use distri_core::agent::AgentOrchestrator;
use distri_types::{Message, MessageRole, ModelSettings, Part};
use std::sync::Arc;

/// Load agent model settings from agent configuration
///
/// If agent_id is provided and found, returns the agent's model_settings.
/// Only StandardAgent types have model_settings; workflow agents return None.
///
/// # Arguments
/// * `executor` - The agent orchestrator to load agents from
/// * `agent_id` - Optional agent ID to load
///
/// # Returns
/// * `Option<ModelSettings>` - Model settings if agent found and is StandardAgent, None otherwise
pub async fn load_agent_model_settings(
    executor: &Arc<AgentOrchestrator>,
    agent_id: Option<&str>,
) -> Option<ModelSettings> {
    let aid = agent_id?;
    if aid == "llm_execute" {
        return None;
    }

    let agent_config = executor.get_agent(aid).await?;

    let def = match &agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(d) => d,
        _ => return None,
    };
    def.model_settings().cloned()
}

/// Load agent configuration and create system message from agent instructions
///
/// If agent_id is provided and found in the agent store, this function will:
/// 1. Load the agent configuration
/// 2. Extract the appropriate instructions/description based on agent type
/// 3. Create a System role message with those instructions
///
/// # Arguments
/// * `executor` - The agent orchestrator to load agents from
/// * `agent_id` - Optional agent ID to load
///
/// # Returns
/// * `Option<Message>` - System message if agent found and has instructions, None otherwise
pub async fn load_agent_system_message(
    executor: &Arc<AgentOrchestrator>,
    agent_id: Option<&str>,
) -> Option<Message> {
    // Only process if agent_id is provided and not the default
    let aid = agent_id?;
    if aid == "llm_execute" {
        return None;
    }

    // Load agent configuration from store
    tracing::info!("Loading system message for agent_id: {}", aid);
    let agent_config = executor.get_agent(aid).await?;
    tracing::info!("Successfully loaded agent config for: {}", aid);

    // Extract instructions based on agent type
    let def = match &agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(d) => d,
        _ => return None,
    };
    let instructions = {
        tracing::info!(
            "Agent '{}' is StandardAgent, instructions length: {}",
            aid,
            def.instructions.len()
        );
        if !def.instructions.is_empty() {
            Some(def.instructions.clone())
        } else {
            None
        }
    };

    // If we have instructions, create a system message
    if instructions.is_none() {
        tracing::warn!("Agent '{}' found but has empty instructions", aid);
    }

    instructions.map(|instructions| {
        tracing::debug!(
            "Creating system message from agent '{}' configuration with {} chars",
            aid,
            instructions.len()
        );

        Message {
            id: uuid::Uuid::new_v4().to_string(),
            name: Some("system".to_string()),
            role: MessageRole::System,
            parts: vec![Part::Text(instructions)],
            created_at: chrono::Utc::now().timestamp_millis(),
            agent_id: None,
            parts_metadata: None,
        }
    })
}
