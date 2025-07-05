use crate::types::{default_agent_version, AgentDefinition, ServerConfig};
use distri_a2a::AgentCard;

pub fn agent_def_to_card(
    def: &AgentDefinition,
    server_config: ServerConfig,
    base_url: &str,
) -> AgentCard {
    AgentCard {
        version: def
            .version
            .clone()
            .unwrap_or_else(|| default_agent_version().unwrap()),
        name: def.name.clone(),
        description: def.description.clone(),
        url: format!("{}/api/v1/agents/{}", base_url, def.name),
        icon_url: def.icon_url.clone(),
        documentation_url: server_config.documentation_url.clone(),
        provider: Some(server_config.agent_provider.clone()),
        preferred_transport: server_config.preferred_transport.clone(),
        capabilities: server_config.capabilities.clone(),
        default_input_modes: server_config.default_input_modes.clone(),
        default_output_modes: server_config.default_output_modes.clone(),
        skills: def.skills.clone(),
        security_schemes: server_config.security_schemes.clone(),
        security: server_config.security.clone(),
    }
}
