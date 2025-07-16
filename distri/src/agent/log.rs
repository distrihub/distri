use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, ContentArrangement, Table};
use tracing::info;

use crate::{
    tools::LlmToolsRegistry,
    types::{LlmDefinition, Message},
};

#[derive(Debug, Clone)]
pub struct ModelLogger {
    pub verbose: bool,
}

impl ModelLogger {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    pub fn log_llm_definition(&self, llm_def: &LlmDefinition, tools_registry: &LlmToolsRegistry) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        table.set_header(vec!["Model", "Settings", "Tools"]);

        let settings_str = format!("{:#?}", llm_def);
        let tools_str = tools_table(tools_registry).to_string();

        table.add_row(vec![llm_def.name.clone(), settings_str, tools_str]);
        tracing::info!("\n{}", table);
    }

    pub fn log_messages(&self, messages: &[Message]) {
        let mut table = Table::new();
        table.set_header(vec!["Role", "Content"]);

        for m in messages {
            let mut content = String::new();
            for c in &m.parts {
                let content_str = match c {
                    crate::types::MessagePart::Text(text) => text.clone(),
                    _ => continue,
                };
                content.push_str(&content_str);
                content.push_str("\n");
            }
            if let Some(metadata) = &m.metadata {
                match metadata {
                    crate::types::MessageMetadata::ToolResponse { tool_call_id, .. } => {
                        content.push_str(&format!("Tool response: {}", tool_call_id));
                    }
                    crate::types::MessageMetadata::ToolCalls { tool_calls } => {
                        content.push_str(&format!("Tool calls: {:?}", tool_calls));
                    }
                    crate::types::MessageMetadata::FinalResponse { final_response } => {
                        content.push_str(&format!("Final response: {}", final_response));
                    }
                    crate::types::MessageMetadata::PlanFacts { facts } => {
                        content.push_str(&format!("Plan facts: {}", facts));
                    }
                    crate::types::MessageMetadata::Plan { plan } => {
                        content.push_str(&format!("Plan: {}", plan));
                    }
                    crate::types::MessageMetadata::ExternalToolCalls { tool_calls, .. } => {
                        content.push_str(&format!("External tool calls: {:?}", tool_calls));
                    }
                    crate::types::MessageMetadata::ToolApprovalRequest { tool_calls, approval_id, .. } => {
                        content.push_str(&format!("Tool approval request: {} (approval_id: {})", tool_calls.len(), approval_id));
                    }
                    crate::types::MessageMetadata::ToolApprovalResponse { approval_id, approved, .. } => {
                        content.push_str(&format!("Tool approval response: {} (approved: {})", approval_id, approved));
                    }
                }
            }
            table.add_row(vec![format!("{:?}", m.role), content]);
        }

        table
            .load_preset(comfy_table::presets::UTF8_FULL)
            .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        tracing::info!("{}", table);
    }
    pub fn log_model_execution(
        &self,
        agent_name: &str,
        model_name: &str,
        messages_count: usize,
        settings: Option<&str>,
        token_usage: Option<u32>,
    ) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        table.set_header(vec![
            "Agent",
            "Model",
            "Messages",
            "Settings",
            "Token Usage",
        ]);

        let settings_str = settings.unwrap_or("None");
        let token_str = token_usage.map_or("None".to_string(), |t| t.to_string());

        table.add_row(vec![
            agent_name,
            model_name,
            &messages_count.to_string(),
            settings_str,
            &token_str,
        ]);

        info!("\n{}", table);
    }
}

pub fn tools_table(tools_registry: &LlmToolsRegistry) -> Table {
    let mut table = Table::new()
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();

    table.add_row(vec!["Server", "Tools"]);
    for (name, tool) in tools_registry.tools.iter() {
        let mut inner_table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_width(60)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        inner_table.add_row(vec!["Tool", "Description"]);

        let description = tool.get_description();
        let description = if description.len() > 60 {
            &description[..60]
        } else {
            &description
        };
        inner_table.add_row(vec![name.clone(), description.to_string()]);
        table.add_row(vec![name.clone(), inner_table.to_string()]);
    }
    table
}
