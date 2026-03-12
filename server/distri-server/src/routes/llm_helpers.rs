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

    let distri_types::configuration::AgentConfig::StandardAgent(def) = &agent_config;
    Some(def.model_settings.clone())
}

/// Merge model settings: base settings are overridden by override settings
/// Only fields that differ from default (sentinel) are considered overrides
pub fn merge_model_settings(
    base: &ModelSettings,
    override_settings: &ModelSettings,
    sentinel: &ModelSettings,
) -> ModelSettings {
    let provider = if std::mem::discriminant(&override_settings.provider)
        != std::mem::discriminant(&sentinel.provider)
    {
        override_settings.provider.clone()
    } else {
        base.provider.clone()
    };

    ModelSettings {
        model: if override_settings.model != sentinel.model {
            override_settings.model.clone()
        } else {
            base.model.clone()
        },
        temperature: override_settings.temperature.or(base.temperature),
        max_tokens: override_settings.max_tokens.or(base.max_tokens),
        context_size: if override_settings.context_size != sentinel.context_size {
            override_settings.context_size
        } else {
            base.context_size
        },
        top_p: override_settings.top_p.or(base.top_p),
        frequency_penalty: override_settings.frequency_penalty.or(base.frequency_penalty),
        presence_penalty: override_settings.presence_penalty.or(base.presence_penalty),
        provider,
        parameters: override_settings.parameters.clone().or(base.parameters.clone()),
        response_format: override_settings
            .response_format
            .clone()
            .or(base.response_format.clone()),
    }
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
    let distri_types::configuration::AgentConfig::StandardAgent(def) = &agent_config;
    let instructions = {
        tracing::info!("Agent '{}' is StandardAgent, instructions length: {}", aid, def.instructions.len());
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
