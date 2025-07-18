pub mod content_filtering;
pub mod logging;
pub mod tool_parsing;

#[cfg(feature = "code")]
pub mod code_parsing;

use std::collections::HashMap;

// Re-export the AgentHooks trait from the main agent module
pub use crate::agent::AgentHooks;

pub use content_filtering::ContentFilteringHooks;
pub use logging::LoggingHooks;
pub use tool_parsing::ToolParsingHooks;

#[cfg(feature = "code")]
pub use code_parsing::CodeParsingHooks;

#[cfg(test)]
mod tests;

fn get_prompts() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("prompts.yaml")).unwrap()
}
