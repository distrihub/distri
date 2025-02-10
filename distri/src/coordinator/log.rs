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

        let border = "+===========================================+";
        let separator = "|----------------------------------------|";

        info!("\n{}", border);
        info!("| Agent: {:<39} |", agent_id);
        info!("{}", separator);

        match step {
            MemoryStep::Task(task) => {
                info!("| Step Type: Task                           |");
                info!("{}", separator);
                info!("| Task:");
                for line in task.task.lines() {
                    info!("| {:<41} |", line);
                }
            }
            MemoryStep::Planning(planning) => {
                info!("| Step Type: Planning                       |");
                info!("{}", separator);
                info!("| Facts:");
                for line in planning.facts.lines() {
                    info!("| {:<41} |", line);
                }
                info!("{}", separator);
                info!("| Plan:");
                for line in planning.plan.lines() {
                    info!("| {:<41} |", line);
                }
            }
            MemoryStep::Action(action) => {
                info!("| Step Type: Action                         |");
                info!("{}", separator);
                if let Some(output) = &action.model_output {
                    info!("| Output:");
                    for line in output.lines() {
                        info!("| {:<41} |", line);
                    }
                }
            }
            MemoryStep::System(system) => {
                info!("| Step Type: System                         |");
                info!("{}", separator);
                info!("| System:");
                for line in system.system_prompt.lines() {
                    info!("| {:<41} |", line);
                }
            }
        }
        info!("{}\n", border);
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

        let border = "+===========================================+";
        let separator = "|----------------------------------------|";

        info!("\n{}", border);
        info!("| Model Execution                            |");
        info!("{}", separator);
        info!("| Model: {:<37} |", model_name);
        info!("| Messages: {:<34} |", messages_count);

        if let Some(settings) = settings {
            info!("{}", separator);
            info!("| Settings:");
            for line in settings.lines() {
                info!("| {:<41} |", line);
            }
        }

        if let Some(tokens) = token_usage {
            info!("{}", separator);
            info!("| Token Usage: {:<31} |", tokens);
        }

        info!("{}\n", border);
    }
}
