mod types;
use std::sync::Arc;

use distri_types::ToolCallFormat;
pub use types::*;
pub mod code;
pub mod formatter;
pub mod scratchpad;
pub mod sequence_store;
pub use scratchpad::format_scratchpad_with_task_filter;
mod unified;
pub use code::CodePlanner;
pub use unified::UnifiedPlanner;

mod simple;
pub use simple::SimplePlanner;

use crate::{types::LlmDefinition, ModelSettings};

pub fn get_planner(agent_def: crate::types::StandardDefinition) -> Arc<dyn PlanningStrategy> {
    let strategy = agent_def
        .strategy
        .as_ref()
        .cloned()
        .unwrap_or(crate::types::AgentStrategy::default());

    // Create a unified planner that adapts behavior based on strategy configuration
    Arc::new(UnifiedPlanner::new(agent_def.clone(), strategy))
}

pub fn get_planning_definition(
    agent_name: String,
    model_settings: ModelSettings,
    tool_format: ToolCallFormat,
) -> LlmDefinition {
    LlmDefinition {
        name: agent_name,
        model_settings: model_settings.clone(),
        tool_format,
    }
}
