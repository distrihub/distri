pub mod tool_parsing;
pub mod logging;
pub mod content_filtering;

// Re-export the AgentHooks trait from the main agent module
pub use crate::agent::AgentHooks;

pub use tool_parsing::ToolParsingHooks;
pub use logging::LoggingHooks;
pub use content_filtering::ContentFilteringHooks;