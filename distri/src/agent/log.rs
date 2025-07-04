use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, ContentArrangement, Table};
use tracing::info;

use crate::{memory::MemoryStep, tools::LlmToolsRegistry, types::LlmDefinition};

#[derive(Debug, Clone)]
pub struct StepLogger {
    pub verbose: bool,
}

impl StepLogger {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    pub fn log_step(&self, agent_id: &str, step: &MemoryStep) {
        if !self.verbose {
            return;
        }

        let mut table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        table.set_header(vec!["Agent", "Step Type", "Details"]);

        match step {
            MemoryStep::Task(task) => {
                let details = task.task.to_string();
                table.add_row(vec![agent_id, "Task", &details]);
            }
            MemoryStep::Planning(planning) => {
                let facts = planning.facts.to_string();
                let plan = planning.plan.to_string();
                table.add_row(vec![agent_id, "Planning", &facts]);
                table.add_row(vec!["", "Plan", &plan]);
            }
            MemoryStep::Action(action) => {
                let output = action.model_output.as_deref().unwrap_or("No output");
                table.add_row(vec![agent_id, "Action", output]);
            }
            MemoryStep::System(system) => {
                let system_prompt = system.system_prompt.to_string();
                table.add_row(vec![agent_id, "System", &system_prompt]);
            }
        }
        info!("\n{}", table);
    }
}

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
        tracing::debug!("\n{}", table);
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
