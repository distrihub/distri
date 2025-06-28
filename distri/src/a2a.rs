use crate::types::{AgentDefinition, ServerConfig};
use distri_a2a::AgentCard;

pub fn agent_def_to_card(
    def: &AgentDefinition,
    server_config: ServerConfig,
    base_url: &str,
) -> AgentCard {
    AgentCard {
        version: distri_a2a::A2A_VERSION.to_string(),
        name: def.name.clone(),
        description: def.description.clone(),
        url: format!("{}/api/v1/agents/{}", base_url, def.name),
        icon_url: def.icon_url.clone(),
        documentation_url: server_config.documentation_url.clone(),
        provider: server_config.provider.clone(),
        preferred_transport: server_config.preferred_transport.clone(),
        capabilities: server_config.capabilities.clone(),
        default_input_modes: server_config.default_input_modes.clone(),
        default_output_modes: server_config.default_output_modes.clone(),
        skills: vec![],
        security_schemes: server_config.security_schemes.clone(),
        security: server_config.security.clone(),
    }
}
