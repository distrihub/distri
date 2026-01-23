use distri_core::agent::AgentOrchestrator;
use distri_types::{Message, MessageRole, Part};
use std::sync::Arc;

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
    let agent_config = executor.get_agent(aid).await?;

    // Extract instructions based on agent type
    let instructions = match &agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(def) => {
            if !def.instructions.is_empty() {
                Some(def.instructions.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
            // For workflow agents, use description as system context
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::CustomAgent(def) => {
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
        }
    };

    // If we have instructions, create a system message
    instructions.map(|instructions| {
        tracing::debug!(
            "Creating system message from agent '{}' configuration",
            aid
        );

        Message {
            id: uuid::Uuid::new_v4().to_string(),
            name: Some("system".to_string()),
            role: MessageRole::System,
            parts: vec![Part::Text(instructions)],
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    })
}
