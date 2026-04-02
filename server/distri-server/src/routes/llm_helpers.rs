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

/// Merge model settings: base settings are overridden by override settings.
/// Individual fields on `override_settings` are considered "explicitly set" when:
/// - `model` is non-empty
/// - `provider` differs from the default (OpenAI)
/// - `context_size` differs from the default
/// - Option fields are Some
pub fn merge_model_settings(
    base: &ModelSettings,
    override_settings: &ModelSettings,
) -> ModelSettings {
    let default_provider = distri_types::ModelProvider::OpenAI {};
    let provider = if std::mem::discriminant(&override_settings.inner.provider)
        != std::mem::discriminant(&default_provider)
    {
        override_settings.inner.provider.clone()
    } else {
        base.inner.provider.clone()
    };

    let default_context_size = 20000u32;
    ModelSettings {
        model: if !override_settings.model.is_empty() {
            override_settings.model.clone()
        } else {
            base.model.clone()
        },
        inner: distri_types::ModelSettingsInner {
            temperature: override_settings
                .inner
                .temperature
                .or(base.inner.temperature),
            max_tokens: override_settings.inner.max_tokens.or(base.inner.max_tokens),
            context_size: if override_settings.inner.context_size != default_context_size {
                override_settings.inner.context_size
            } else {
                base.inner.context_size
            },
            top_p: override_settings.inner.top_p.or(base.inner.top_p),
            frequency_penalty: override_settings
                .inner
                .frequency_penalty
                .or(base.inner.frequency_penalty),
            presence_penalty: override_settings
                .inner
                .presence_penalty
                .or(base.inner.presence_penalty),
            provider,
            parameters: override_settings
                .inner
                .parameters
                .clone()
                .or(base.inner.parameters.clone()),
            response_format: override_settings
                .inner
                .response_format
                .clone()
                .or(base.inner.response_format.clone()),
            api_format: if override_settings.inner.api_format != distri_types::OpenAiApiFormat::Auto
            {
                override_settings.inner.api_format.clone()
            } else {
                base.inner.api_format.clone()
            },
        },
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
