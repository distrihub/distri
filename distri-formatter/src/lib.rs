pub mod state;
pub mod text;

use distri_types::{AgentEvent, ToolResponse};

/// Trait for formatting agent events into output.
/// Implementations decide the output format (terminal ANSI, plain text, HTML, etc.)
pub trait Formatter: Send + Sync {
    /// Handle an agent event (text delta, tool start, step start, etc.)
    fn handle_event(&mut self, event: &AgentEvent);

    /// Format a tool result for display
    fn format_tool_result(&self, result: &ToolResponse) -> Option<String>;

    /// Get any accumulated final text content
    fn final_content(&self) -> String;

    /// Get the thread ID if captured from events
    fn thread_id(&self) -> Option<String>;
}
