//! Plain-text formatter suitable for Telegram, WhatsApp, logs, etc.
//!
//! Handles the same events as `EventPrinter` but produces plain text
//! with no ANSI codes, no spinners, no terminal manipulation, no image rendering.

use chrono::Local;
use distri_types::{AgentEvent, AgentEventType, ToolResponse};

use crate::state::{
    ChatState, MessageState, StepState, ToolCallState, ToolCallStatus,
    format_tool_call, is_probe_call,
};
use crate::{Formatter, RendererOutput};

/// A plain-text event formatter.
///
/// Accumulates output into an internal `String` buffer.
/// Retrieve the accumulated text via [`Formatter::final_content`].
pub struct TextFormatter {
    state: ChatState,
    /// Accumulated output text.
    output: String,
    /// Whether to show tool start/result lines.
    show_tools: bool,
    /// Display name override for the agent.
    agent_name: Option<String>,
}

impl Default for TextFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl TextFormatter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
            output: String::new(),
            show_tools: true,
            agent_name: None,
        }
    }

    pub fn with_show_tools(mut self, show: bool) -> Self {
        self.show_tools = show;
        self
    }

    pub fn with_agent_name(mut self, name: String) -> Self {
        self.agent_name = Some(name);
        self
    }

    /// Append a line to the output buffer (with trailing newline).
    fn push_line(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');
    }

    /// Append text without a trailing newline.
    fn push(&mut self, text: &str) {
        self.output.push_str(text);
    }
}

impl Formatter for TextFormatter {
    fn handle_event(&mut self, event: &AgentEvent) {
        if !self.state.printed_header {
            self.state.printed_header = true;
        }

        // Capture thread_id from the first event that has one.
        if self.state.thread_id.is_none() && !event.thread_id.is_empty() {
            self.state.thread_id = Some(event.thread_id.clone());
        }

        // Track agent changes.
        let agent_changed = self
            .state
            .current_agent
            .as_ref()
            .map(|a| a != &event.agent_id)
            .unwrap_or(true);
        if agent_changed && !event.agent_id.is_empty() {
            if self.state.current_agent.is_some() {
                self.push_line(&format!("=> Agent: {}", event.agent_id));
            }
            self.state.current_agent = Some(event.agent_id.clone());
        }

        match &event.event {
            AgentEventType::PlanStarted { .. } => {
                self.state.is_planning = true;
            }
            AgentEventType::PlanFinished { .. } => {
                self.state.is_planning = false;
            }
            AgentEventType::StepStarted {
                step_id,
                step_index,
            } => {
                self.state.steps.insert(
                    step_id.clone(),
                    StepState {
                        id: step_id.clone(),
                        title: format!("Step {}", step_index + 1),
                        index: *step_index,
                        status: "running".into(),
                    },
                );
            }
            AgentEventType::StepCompleted { step_id, success } => {
                let fail_msg = if let Some(step) = self.state.steps.get_mut(step_id) {
                    step.status = if *success {
                        "done".into()
                    } else {
                        "error".into()
                    };
                    if !success {
                        Some(format!("Step {} failed", step.index + 1))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(msg) = fail_msg {
                    self.push_line(&msg);
                }
            }
            AgentEventType::TextMessageStart {
                message_id, role, ..
            } => {
                let message = MessageState {
                    id: message_id.to_string(),
                    role: role.clone(),
                    content: String::new(),
                    is_streaming: true,
                    is_complete: false,
                    step_id: None,
                };
                self.state.messages.insert(message_id.to_string(), message);
                self.state.current_message_id = Some(message_id.to_string());

                // No header prefix in plain text — content follows directly.
            }
            AgentEventType::TextMessageContent {
                message_id, delta, ..
            } => {
                if let Some(msg) = self.state.messages.get_mut(message_id) {
                    msg.content.push_str(delta);
                    self.push(delta);
                }
            }
            AgentEventType::TextMessageEnd { message_id, .. } => {
                if let Some(msg) = self.state.messages.get_mut(message_id) {
                    msg.is_streaming = false;
                    msg.is_complete = true;
                }
                if self
                    .state
                    .current_message_id
                    .as_ref()
                    .map(|id| id == message_id)
                    .unwrap_or(false)
                {
                    self.state.current_message_id = None;
                }
                // Ensure trailing newline after message.
                if !self.output.ends_with('\n') {
                    self.output.push('\n');
                }
            }
            AgentEventType::ToolExecutionStart {
                tool_call_id,
                tool_call_name,
                input,
                ..
            } => {
                if self.show_tools && !is_probe_call(tool_call_name, input) {
                    self.push_line(&format!(
                        "Using {}",
                        format_tool_call(tool_call_name, input)
                    ));
                }
                self.state.tool_calls.insert(
                    tool_call_id.to_string(),
                    ToolCallState {
                        tool_call_id: tool_call_id.to_string(),
                        tool_name: tool_call_name.to_string(),
                        input: input.clone(),
                        status: ToolCallStatus::Running,
                        result: None,
                        error: None,
                    },
                );
            }
            AgentEventType::ToolExecutionEnd {
                tool_call_id,
                success,
                ..
            } => {
                if let Some(tc) = self.state.tool_calls.get_mut(tool_call_id) {
                    tc.status = if *success {
                        ToolCallStatus::Completed
                    } else {
                        ToolCallStatus::Error
                    };
                }
            }
            AgentEventType::ToolResults { results, .. } => {
                if self.show_tools {
                    for result in results {
                        if let Some(formatted) = self.format_tool_result(result) {
                            self.push_line(&formatted);
                        }
                    }
                }
            }
            AgentEventType::ToolCalls { .. } => {
                // Suppressed — individual ToolExecutionStart events show each call.
            }
            AgentEventType::RunFinished { success, .. } => {
                if !success {
                    self.push_line("Run completed with errors");
                }
            }
            AgentEventType::RunError { message, code } => {
                let stamp = Local::now().format("%H:%M:%S").to_string();
                self.push_line(&format!(
                    "{} [{}] run failed: {} ({:?})",
                    stamp, event.agent_id, message, code
                ));
            }
            AgentEventType::InlineHookRequested { request } => {
                self.push_line(&format!(
                    "Awaiting inline hook {} for {}",
                    request.hook_id,
                    request.hook,
                ));
            }
            AgentEventType::TodosUpdated {
                formatted_todos, ..
            } => {
                self.push_line(&format!("Todos updated:\n{}", formatted_todos));
            }
            AgentEventType::BrowserScreenshot { .. } => {
                // No image rendering in plain text.
                self.push_line("[Browser screenshot]");
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                let reason_str = reason
                    .as_deref()
                    .map(|r| format!(" ({})", r))
                    .unwrap_or_default();
                self.push_line(&format!(
                    "Transferring: {} -> {}{}",
                    from_agent, to_agent, reason_str
                ));
            }
            _ => {}
        }
    }

    fn format_tool_result(&self, result: &ToolResponse) -> Option<String> {
        // Simple plain-text summary: show tool name and first text part.
        let text = result
            .parts
            .iter()
            .filter_map(|p| match p {
                distri_types::Part::Text(t) => Some(t.as_str()),
                distri_types::Part::Data(_) => {
                    Some("") // skip data parts in plain text
                }
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        if text.is_empty() {
            None
        } else {
            // Truncate long results
            let preview = if text.len() > 200 {
                format!("{}...", &text[..200])
            } else {
                text
            };
            Some(format!("  Result ({}): {}", result.tool_name, preview))
        }
    }

    fn final_content(&self) -> String {
        self.output.clone()
    }

    fn thread_id(&self) -> Option<String> {
        self.state.thread_id.clone()
    }

    fn take_output(&mut self) -> RendererOutput {
        if self.output.is_empty() {
            RendererOutput::None
        } else {
            RendererOutput::Text(std::mem::take(&mut self.output))
        }
    }

    fn clear_content(&mut self) {
        self.output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::{AgentEvent, AgentEventType, MessageRole};

    fn make_event(event: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "thread-1".into(),
            run_id: "run-1".into(),
            event,
            task_id: "task-1".into(),
            agent_id: "test-agent".into(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    #[test]
    fn text_message_accumulates() {
        let mut fmt = TextFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::TextMessageStart {
            message_id: "m1".into(),
            step_id: "s1".into(),
            role: MessageRole::Assistant,
            is_final: None,
        }));
        fmt.handle_event(&make_event(AgentEventType::TextMessageContent {
            message_id: "m1".into(),
            step_id: "s1".into(),
            delta: "Hello ".into(),
            stripped_content: None,
        }));
        fmt.handle_event(&make_event(AgentEventType::TextMessageContent {
            message_id: "m1".into(),
            step_id: "s1".into(),
            delta: "world!".into(),
            stripped_content: None,
        }));
        fmt.handle_event(&make_event(AgentEventType::TextMessageEnd {
            message_id: "m1".into(),
            step_id: "s1".into(),
        }));

        assert_eq!(fmt.final_content(), "Hello world!\n");
    }

    #[test]
    fn thread_id_captured() {
        let mut fmt = TextFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::RunStarted {}));
        assert_eq!(fmt.thread_id(), Some("thread-1".into()));
    }

    #[test]
    fn tool_call_shown() {
        let mut fmt = TextFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "search".into(),
            input: serde_json::json!({"query": "rust async"}),
        }));
        assert!(fmt.final_content().contains("Using search(\"rust async\")"));
    }

    #[test]
    fn probe_calls_hidden() {
        let mut fmt = TextFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "load_skill".into(),
            input: serde_json::json!({"skill_name": "?"}),
        }));
        assert!(fmt.final_content().is_empty());
    }
}
