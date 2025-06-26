use crate::types::{AgentDefinition, ServerConfig, ServerTools};
use distri_a2a::{AgentCard, AgentSkill};

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
        skills: extract_skills_from_agent(def),
        security_schemes: server_config.security_schemes.clone(),
        security: server_config.security.clone(),
    }
}

pub fn agent_def_to_card_with_tools(
    def: &AgentDefinition,
    server_config: ServerConfig,
    base_url: &str,
    tools: &[ServerTools],
) -> AgentCard {
    let mut card = agent_def_to_card(def, server_config, base_url);
    card.skills.extend(extract_skills_from_tools(tools));
    card
}

fn extract_skills_from_agent(def: &AgentDefinition) -> Vec<AgentSkill> {
    let mut skills = vec![];

    // Create a basic conversational skill
    skills.push(AgentSkill {
        id: format!("{}_conversation", def.name),
        name: "Conversation".to_string(),
        description: format!("Engage in conversation and answer questions. {}", def.description),
        tags: vec!["conversation".to_string(), "chat".to_string(), "assistance".to_string()],
        examples: vec![
            "Ask me anything and I'll help you with your questions.".to_string(),
            "I can provide explanations, analysis, and assistance.".to_string(),
        ],
        input_modes: None,
        output_modes: None,
    });

    // Add planning skill if planning is enabled
    if def.plan.is_some() {
        skills.push(AgentSkill {
            id: format!("{}_planning", def.name),
            name: "Task Planning".to_string(),
            description: "Break down complex tasks into actionable steps and create execution plans.".to_string(),
            tags: vec!["planning".to_string(), "strategy".to_string(), "workflow".to_string()],
            examples: vec![
                "I can create a step-by-step plan for complex projects.".to_string(),
                "Let me break that down into manageable tasks.".to_string(),
            ],
            input_modes: None,
            output_modes: None,
        });
    }

    // Add analysis skill based on system prompt content
    if let Some(system_prompt) = &def.system_prompt {
        if system_prompt.to_lowercase().contains("analy") || 
           system_prompt.to_lowercase().contains("research") ||
           system_prompt.to_lowercase().contains("expert") {
            skills.push(AgentSkill {
                id: format!("{}_analysis", def.name),
                name: "Analysis & Research".to_string(),
                description: "Provide detailed analysis, research, and expert insights on various topics.".to_string(),
                tags: vec!["analysis".to_string(), "research".to_string(), "expertise".to_string()],
                examples: vec![
                    "I can analyze complex topics and provide detailed insights.".to_string(),
                    "Let me research that topic and provide a comprehensive analysis.".to_string(),
                ],
                input_modes: None,
                output_modes: None,
            });
        }

        if system_prompt.to_lowercase().contains("code") || 
           system_prompt.to_lowercase().contains("program") ||
           system_prompt.to_lowercase().contains("develop") {
            skills.push(AgentSkill {
                id: format!("{}_coding", def.name),
                name: "Code Development".to_string(),
                description: "Assist with programming, code review, debugging, and software development.".to_string(),
                tags: vec!["coding".to_string(), "programming".to_string(), "development".to_string()],
                examples: vec![
                    "I can help you write, debug, and optimize code.".to_string(),
                    "Let me review your code and suggest improvements.".to_string(),
                ],
                input_modes: None,
                output_modes: None,
            });
        }
    }

    skills
}

fn extract_skills_from_tools(tools: &[ServerTools]) -> Vec<AgentSkill> {
    let mut skills = vec![];

    for server_tool in tools {
        for tool in &server_tool.tools {
            // Create a skill for each tool
            skills.push(AgentSkill {
                id: format!("tool_{}", tool.name.replace(' ', "_").to_lowercase()),
                name: tool.name.clone(),
                description: tool.description.clone().unwrap_or_else(|| {
                    format!("Use the {} tool to perform specific operations.", tool.name)
                }),
                tags: vec![
                    "tool".to_string(),
                    "automation".to_string(),
                    server_tool.definition.name.clone(),
                ],
                examples: vec![
                    format!("I can use {} to help you with specific tasks.", tool.name),
                ],
                input_modes: Some(vec!["text/plain".to_string()]),
                output_modes: Some(vec!["text/plain".to_string()]),
            });
        }

        // Add a general skill for the MCP server
        if !server_tool.tools.is_empty() {
            skills.push(AgentSkill {
                id: format!("mcp_{}", server_tool.definition.name.replace(' ', "_").to_lowercase()),
                name: format!("{} Integration", server_tool.definition.name),
                description: format!(
                    "Access and utilize {} capabilities with {} available tools.",
                    server_tool.definition.name,
                    server_tool.tools.len()
                ),
                tags: vec![
                    "integration".to_string(),
                    "mcp".to_string(),
                    server_tool.definition.name.clone(),
                ],
                examples: vec![
                    format!("I can integrate with {} to expand my capabilities.", server_tool.definition.name),
                ],
                input_modes: Some(vec!["text/plain".to_string()]),
                output_modes: Some(vec!["text/plain".to_string(), "application/json".to_string()]),
            });
        }
    }

    skills
}
