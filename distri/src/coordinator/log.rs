use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, ContentArrangement, Table};
use tracing::info;

use crate::memory::MemoryStep;

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

    pub fn log_model_execution(
        &self,
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
        table.set_header(vec!["Model", "Messages", "Settings", "Token Usage"]);

        let settings_str = settings.unwrap_or("None");
        let token_str = token_usage.map_or("None".to_string(), |t| t.to_string());

        table.add_row(vec![
            model_name,
            &messages_count.to_string(),
            settings_str,
            &token_str,
        ]);

        info!("\n{}", table);
    }
}
