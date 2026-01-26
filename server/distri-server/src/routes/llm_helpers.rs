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

    match &agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(def) => {
            Some(def.model_settings.clone())
        }
        _ => None,
    }
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
        temperature: if (override_settings.temperature - sentinel.temperature).abs() > f32::EPSILON
        {
            override_settings.temperature
        } else {
            base.temperature
        },
        max_tokens: if override_settings.max_tokens != sentinel.max_tokens {
            override_settings.max_tokens
        } else {
            base.max_tokens
        },
        context_size: if override_settings.context_size != sentinel.context_size {
            override_settings.context_size
        } else {
            base.context_size
        },
        top_p: if (override_settings.top_p - sentinel.top_p).abs() > f32::EPSILON {
            override_settings.top_p
        } else {
            base.top_p
        },
        frequency_penalty: if (override_settings.frequency_penalty - sentinel.frequency_penalty)
            .abs()
            > f32::EPSILON
        {
            override_settings.frequency_penalty
        } else {
            base.frequency_penalty
        },
        presence_penalty: if (override_settings.presence_penalty - sentinel.presence_penalty).abs()
            > f32::EPSILON
        {
            override_settings.presence_penalty
        } else {
            base.presence_penalty
        },
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
    let instructions = match &agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(def) => {
            tracing::info!("Agent '{}' is StandardAgent, instructions length: {}", aid, def.instructions.len());
            if !def.instructions.is_empty() {
                Some(def.instructions.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
            tracing::info!("Agent '{}' is SequentialWorkflowAgent, description length: {}", aid, def.description.len());
            // For workflow agents, use description as system context
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
            tracing::info!("Agent '{}' is DagWorkflowAgent, description length: {}", aid, def.description.len());
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
        }
        distri_types::configuration::AgentConfig::CustomAgent(def) => {
            tracing::info!("Agent '{}' is CustomAgent, description length: {}", aid, def.description.len());
            if !def.description.is_empty() {
                Some(def.description.clone())
            } else {
                None
            }
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
        }
    })
}
