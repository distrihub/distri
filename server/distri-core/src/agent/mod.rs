pub mod agent_loop;
pub mod browser_sessions;
pub mod context;
pub mod context_size_manager;
pub mod debug;
pub mod file;
pub mod hooks;
pub mod log;
pub mod memory;
pub mod orchestrator;
mod parser;
pub mod plugin_registry;
mod plugin_storage;
pub mod prompt_registry {
    pub use distri_types::prompt::*;
}
pub mod prompt_validation;
pub mod reflection;
pub mod server;
pub mod standard;
pub mod strategy;
pub mod todos;
pub mod token_estimator;
pub mod tool_lookup;
pub mod types;
pub mod workflow;
// Export specific items to avoid conflicts
pub use agent_loop::*;
pub use distri_types::parse_agent_markdown_content;
pub use parser::load_agents_from_dir;
pub use prompt_validation::{
    builtin_partials, extract_partial_references, format_validation_table, validate_agent_prompt,
    validate_agent_prompt_with_partials, validate_partial_references, Criticality, ValidationIssue,
};
pub use standard::*;
pub use workflow::*;
// Don't export AgentHooks from types to avoid conflict
pub use types::{AgentEvent, AgentEventType, BaseAgent, ExecutorContext, InvokeResult};

// Export orchestrator types
pub use orchestrator::AgentOrchestrator;

// Export DAP registry
pub use plugin_registry::{PluginOptions, PluginRegistry};
pub use plugin_storage::InMemoryPluginResolver;

// Export prompt registry
pub use prompt_registry::{PromptRegistry, PromptSection, PromptTemplate, TemplateData};

// Export types from types module
pub use types::{AgentType, CoordinatorMessage};

// Export strategy types
pub use strategy::{
    AgentExecutor, CodePlanner, ExecutionStrategy, PlanningStrategy, UnifiedPlanner,
};
