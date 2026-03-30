//! WhatsApp surface renderer — produces basic formatted output
//! with status coalescing, code blocks, and media support.
//!
//! WhatsApp doesn't support message editing, so status updates are sent as
//! separate messages. The renderer coalesces rapid status updates via a
//! debounce window tracked by the gateway sender.

use distri_types::{AgentEvent, AgentEventType, ToolResponse};

use crate::state::{
    is_probe_call, ChatState, MessageState, StepState, ToolCallState,
    ToolCallStatus,
};
use crate::status::format_status_text;
use crate::{Formatter, MediaAttachment, ParseMode, RendererOutput, SurfaceRenderer};

/// Maximum message length for WhatsApp (65K, rarely hit).
const WHATSAPP_MAX_LEN: usize = 65_000;

/// WhatsApp surface renderer.
///
/// Produces WhatsApp-native formatting (*bold*, _italic_, ```code```).
/// Status updates and final text are separate outputs — the gateway sender
/// decides when/how to send them.
pub struct WhatsAppRenderer {
    /// Accumulated text output.
    output: String,
    /// Pending media attachments.
    media: Vec<MediaAttachment>,
    /// Whether there is new output to flush.
    dirty: bool,
    /// Current status text (latest tool action).
    current_status: Option<String>,
    /// Whether the status has changed since last take_output.
    status_dirty: bool,
}

impl WhatsAppRenderer {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            media: Vec::new(),
            dirty: false,
            current_status: None,
            status_dirty: false,
        }
    }
}

impl Default for WhatsAppRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceRenderer for WhatsAppRenderer {
    fn render_text(&mut self, content: &str) {
        self.output.push_str(content);
        self.dirty = true;
    }

    fn render_markdown(&mut self, md: &str) {
        // WhatsApp supports a subset: *bold*, _italic_, ```code```
        // Pass through as-is — the WhatsApp API handles native formatting.
        self.output.push_str(md);
        self.dirty = true;
    }

    fn render_code_block(&mut self, code: &str, _lang: Option<&str>) {
        // WhatsApp only supports triple backtick, no language hint.
        self.output
            .push_str(&format!("```\n{}\n```\n", code));
        self.dirty = true;
    }

    fn render_diff(&mut self, diff: &str) {
        // Render as monospace code block with +/- prefixes.
        self.render_code_block(diff, None);
    }

    fn render_tool_start(&mut self, name: &str, _input: &serde_json::Value, status_text: &str) {
        self.current_status = Some(status_text.to_string());
        self.status_dirty = true;
        tracing::debug!(tool = name, status = status_text, "whatsapp tool start");
    }

    fn render_tool_result(&mut self, _name: &str, _result: &ToolResponse, _verbose: bool) {
        // WhatsApp doesn't show individual tool results.
    }

    fn render_status_update(&mut self, text: &str) {
        self.current_status = Some(text.to_string());
        self.status_dirty = true;
    }

    fn render_image(&mut self, data: &[u8], mime: &str) {
        self.media.push(MediaAttachment {
            data: data.to_vec(),
            mime_type: mime.to_string(),
            filename: None,
        });
        self.dirty = true;
    }

    fn show_planning(&mut self, _phrase: &str) {
        // WhatsApp uses typing indicator — handled by the gateway sender.
    }

    fn clear_planning(&mut self) {}

    fn render_agent_transfer(&mut self, _from: &str, to: &str, reason: Option<&str>) {
        let reason_str = reason
            .map(|r| format!(" ({})", r))
            .unwrap_or_default();
        let status = format!("Handing off to {}{}...", to, reason_str);
        self.current_status = Some(status);
        self.status_dirty = true;
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_rich_text(&self) -> bool {
        false // WhatsApp has very limited formatting
    }

    fn max_message_length(&self) -> Option<usize> {
        Some(WHATSAPP_MAX_LEN)
    }

    fn take_output(&mut self) -> RendererOutput {
        // Priority: final text > status > media-only
        if self.dirty && !self.output.is_empty() {
            self.dirty = false;
            self.status_dirty = false;
            let text = self.output.clone();
            let media = std::mem::take(&mut self.media);

            if text.len() > WHATSAPP_MAX_LEN {
                let chunks = split_message(&text, WHATSAPP_MAX_LEN);
                let parts: Vec<RendererOutput> = chunks
                    .into_iter()
                    .map(|chunk| RendererOutput::RichText {
                        text: chunk,
                        parse_mode: ParseMode::Plain,
                        media: Vec::new(),
                    })
                    .collect();
                return RendererOutput::Chunks(parts);
            }

            return RendererOutput::RichText {
                text,
                parse_mode: ParseMode::Plain,
                media,
            };
        }

        if self.status_dirty {
            self.status_dirty = false;
            if let Some(status) = &self.current_status {
                return RendererOutput::RichText {
                    text: status.clone(),
                    parse_mode: ParseMode::Plain,
                    media: Vec::new(),
                };
            }
        }

        if self.dirty && !self.media.is_empty() {
            self.dirty = false;
            let media = std::mem::take(&mut self.media);
            return RendererOutput::RichText {
                text: String::new(),
                parse_mode: ParseMode::Plain,
                media,
            };
        }

        RendererOutput::None
    }
}

// ---------------------------------------------------------------------------
// WhatsAppFormatter — combines Formatter + WhatsAppRenderer
// ---------------------------------------------------------------------------

/// Full WhatsApp formatter: shared event state machine + WhatsApp rendering.
pub struct WhatsAppFormatter {
    state: ChatState,
    renderer: WhatsAppRenderer,
    agent_name: Option<String>,
}

impl WhatsAppFormatter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
            renderer: WhatsAppRenderer::new(),
            agent_name: None,
        }
    }

    pub fn with_agent_name(mut self, name: String) -> Self {
        self.agent_name = Some(name);
        self
    }

    /// Get a reference to the surface renderer.
    pub fn surface_renderer(&mut self) -> &mut WhatsAppRenderer {
        &mut self.renderer
    }
}

impl Default for WhatsAppFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter for WhatsAppFormatter {
    fn handle_event(&mut self, event: &AgentEvent) {
        // Capture thread_id.
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
            self.state.current_agent = Some(event.agent_id.clone());
        }

        match &event.event {
            AgentEventType::PlanStarted { .. } => {
                self.state.is_planning = true;
                self.renderer.show_planning("Planning...");
            }
            AgentEventType::PlanFinished { .. } => {
                self.state.is_planning = false;
                self.renderer.clear_planning();
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
                if let Some(step) = self.state.steps.get_mut(step_id) {
                    step.status = if *success { "done" } else { "error" }.into();
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
            }
            AgentEventType::TextMessageContent {
                message_id, delta, ..
            } => {
                if let Some(msg) = self.state.messages.get_mut(message_id) {
                    msg.content.push_str(delta);
                }
                self.renderer.render_text(delta);
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
            }
            AgentEventType::ToolExecutionStart {
                tool_call_id,
                tool_call_name,
                input,
                ..
            } => {
                if !is_probe_call(tool_call_name, input) {
                    let status_text = format_status_text(tool_call_name, input);
                    self.renderer
                        .render_tool_start(tool_call_name, input, &status_text);
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
                for result in results {
                    let text_content: String = result
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            distri_types::Part::Text(t) => Some(t.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    if looks_like_diff(&text_content) {
                        self.renderer.render_diff(&text_content);
                    }
                }
            }
            AgentEventType::ToolCalls { .. } => {}
            AgentEventType::RunFinished { .. } => {}
            AgentEventType::RunError { message, .. } => {
                self.renderer
                    .render_text(&format!("Error: {}", message));
            }
            AgentEventType::BrowserScreenshot { image, .. } => {
                if self.renderer.supports_images() {
                    self.renderer.render_image(image.as_bytes(), "image/png");
                }
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                self.renderer
                    .render_agent_transfer(from_agent, to_agent, reason.as_deref());
            }
            _ => {}
        }
    }

    fn format_tool_result(&self, result: &ToolResponse) -> Option<String> {
        let text = result
            .parts
            .iter()
            .filter_map(|p| match p {
                distri_types::Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn final_content(&self) -> String {
        self.renderer.output.clone()
    }

    fn thread_id(&self) -> Option<String> {
        self.state.thread_id.clone()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if text content looks like a unified diff.
fn looks_like_diff(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().take(10).collect();
    let diff_indicators = lines
        .iter()
        .filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with("@@"))
        .count();
    diff_indicators >= 2
}

/// Split a message into chunks at paragraph/sentence boundaries.
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining[..max_len]
            .rfind("\n\n")
            .or_else(|| remaining[..max_len].rfind('\n'))
            .or_else(|| remaining[..max_len].rfind(". "))
            .unwrap_or(max_len);

        let split_at = if split_at == 0 { max_len } else { split_at };

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }

    chunks
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
    fn status_on_tool_start() {
        let mut fmt = WhatsAppFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "execute_shell".into(),
            input: serde_json::json!({"command": "npm test"}),
        }));

        match fmt.surface_renderer().take_output() {
            RendererOutput::RichText { text, .. } => {
                assert_eq!(text, "Running command: npm test");
            }
            other => panic!("Expected RichText status, got {:?}", other),
        }
    }

    #[test]
    fn text_accumulates() {
        let mut fmt = WhatsAppFormatter::new();
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

        assert_eq!(fmt.final_content(), "Hello world!");
    }
}
