//! Telegram surface renderer — produces HTML-formatted output with message
//! splitting, code blocks, diffs, and image support.
//!
//! Output is escaped + linkified via [`crate::telegram_html`] before being
//! emitted as `RendererOutput::RichText { parse_mode: ParseMode::Html }`.
//! HTML is the safer default for raw LLM text (only `< > &` need escaping)
//! whereas MarkdownV2 has 18 metacharacters that all need escaping correctly
//! or the message silently falls back to plain. Callers that want MarkdownV2
//! can construct a `Reply::markdown_v2(...)` directly via the gateway types.

use distri_types::{AgentEvent, AgentEventType, ToolResponse};

use crate::state::{
    ChatState, MessageState, StepState, ToolCallState, ToolCallStatus, is_probe_call,
};
use crate::status::format_status_text;
use crate::telegram_html::escape_and_linkify;
use crate::{Formatter, MediaAttachment, ParseMode, RendererOutput, SurfaceRenderer};

/// Maximum message length for Telegram (leave margin for formatting).
const TELEGRAM_MAX_LEN: usize = 4000;

/// Telegram surface renderer.
///
/// Produces MarkdownV2-formatted output with status updates that can be
/// edited in-place via the Telegram Bot API.
pub struct TelegramRenderer {
    /// Accumulated text output for the current message.
    output: String,
    /// Pending media attachments.
    media: Vec<MediaAttachment>,
    /// Whether there is new output to flush.
    dirty: bool,
    /// Current status text (shown while agent is working).
    current_status: Option<String>,
    /// Whether we are in status-only mode (no final text yet).
    in_status_mode: bool,
}

impl TelegramRenderer {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            media: Vec::new(),
            dirty: false,
            current_status: None,
            in_status_mode: true,
        }
    }
}

impl Default for TelegramRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceRenderer for TelegramRenderer {
    fn render_text(&mut self, content: &str) {
        self.in_status_mode = false;
        self.current_status = None;
        self.output.push_str(content);
        self.dirty = true;
    }

    fn render_markdown(&mut self, md: &str) {
        self.in_status_mode = false;
        self.current_status = None;
        self.output.push_str(md);
        self.dirty = true;
    }

    fn render_code_block(&mut self, code: &str, lang: Option<&str>) {
        let lang_hint = lang.unwrap_or("");
        self.output
            .push_str(&format!("```{}\n{}\n```\n", lang_hint, code));
        self.dirty = true;
    }

    fn render_diff(&mut self, diff: &str) {
        self.render_code_block(diff, Some("diff"));
    }

    fn render_tool_start(&mut self, name: &str, _input: &serde_json::Value, status_text: &str) {
        self.current_status = Some(status_text.to_string());
        self.dirty = true;
        tracing::debug!(tool = name, status = status_text, "telegram tool start");
    }

    fn render_tool_result(&mut self, _name: &str, _result: &ToolResponse, _verbose: bool) {
        // In Telegram, tool results are not shown inline — the status just
        // gets replaced by the next status or final text.
    }

    fn render_status_update(&mut self, text: &str) {
        self.current_status = Some(text.to_string());
        self.dirty = true;
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
        // Telegram uses sendChatAction(typing) for planning — handled by the
        // gateway sender, not the renderer.
    }

    fn clear_planning(&mut self) {}

    fn render_agent_transfer(&mut self, _from: &str, to: &str, reason: Option<&str>) {
        let reason_str = reason.map(|r| format!(" ({})", r)).unwrap_or_default();
        let status = format!("Handing off to {}{}...", to, reason_str);
        self.current_status = Some(status);
        self.dirty = true;
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_rich_text(&self) -> bool {
        true
    }

    fn max_message_length(&self) -> Option<usize> {
        Some(TELEGRAM_MAX_LEN)
    }

    fn take_output(&mut self) -> RendererOutput {
        if !self.dirty {
            return RendererOutput::None;
        }
        self.dirty = false;

        // If we have final text content, return it (possibly with media).
        if !self.output.is_empty() {
            // Escape `< > &` and wrap bare URLs in <a href="…">. Done once
            // here so the chunk-splitter below sees the final HTML byte
            // length, not the raw LLM text length.
            let text = escape_and_linkify(&self.output);
            let media = std::mem::take(&mut self.media);

            // Split long messages at paragraph boundaries.
            if text.len() > TELEGRAM_MAX_LEN {
                let chunks = split_message(&text, TELEGRAM_MAX_LEN);
                let mut parts: Vec<RendererOutput> = chunks
                    .into_iter()
                    .map(|chunk| RendererOutput::RichText {
                        text: chunk,
                        parse_mode: ParseMode::Html,
                        media: Vec::new(),
                    })
                    .collect();
                // Attach media to last chunk.
                if !media.is_empty()
                    && let Some(RendererOutput::RichText { media: m, .. }) = parts.last_mut()
                {
                    *m = media;
                }
                return RendererOutput::Chunks(parts);
            }

            return RendererOutput::RichText {
                text,
                parse_mode: ParseMode::Html,
                media,
            };
        }

        // Status-only output (no final text yet).
        if let Some(status) = &self.current_status {
            return RendererOutput::RichText {
                text: status.clone(),
                parse_mode: ParseMode::Plain,
                media: Vec::new(),
            };
        }

        // Only media with no text.
        if !self.media.is_empty() {
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
// TelegramFormatter — combines Formatter + TelegramRenderer
// ---------------------------------------------------------------------------

/// Full Telegram formatter: shared event state machine + Telegram rendering.
pub struct TelegramFormatter {
    state: ChatState,
    renderer: TelegramRenderer,
    /// Display name for the agent.
    agent_name: Option<String>,
}

impl TelegramFormatter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
            renderer: TelegramRenderer::new(),
            agent_name: None,
        }
    }

    pub fn with_agent_name(mut self, name: String) -> Self {
        self.agent_name = Some(name);
        self
    }

    /// Get a reference to the surface renderer.
    pub fn surface_renderer(&mut self) -> &mut TelegramRenderer {
        &mut self.renderer
    }
}

impl Default for TelegramFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter for TelegramFormatter {
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
            AgentEventType::StepCompleted {
                step_id, success, ..
            } => {
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
                // Skip probe calls.
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
            AgentEventType::ToolResults { .. } => {
                // Tool results are not shown in Telegram — the status gets
                // replaced by the next status or the final assistant text.
                // Rendering tool results here would pollute the output with
                // skill content, API responses, etc.
            }
            AgentEventType::ToolCalls { .. } => {}
            AgentEventType::RunFinished { .. } => {}
            AgentEventType::RunError { message, .. } => {
                self.renderer.render_text(&format!("Error: {}", message));
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

        if text.is_empty() { None } else { Some(text) }
    }

    fn final_content(&self) -> String {
        self.renderer.output.clone()
    }

    fn thread_id(&self) -> Option<String> {
        self.state.thread_id.clone()
    }

    fn take_output(&mut self) -> RendererOutput {
        self.renderer.take_output()
    }

    fn clear_content(&mut self) {
        self.renderer.output.clear();
        self.renderer.dirty = false;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    fn status_text_on_tool_start() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "search".into(),
            input: serde_json::json!({"query": "rust async"}),
        }));

        match fmt.surface_renderer().take_output() {
            RendererOutput::RichText { text, .. } => {
                assert_eq!(text, "Searching: rust async");
            }
            other => panic!("Expected RichText status, got {:?}", other),
        }
    }

    #[test]
    fn text_message_produces_rich_output() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::TextMessageStart {
            message_id: "m1".into(),
            step_id: "s1".into(),
            role: MessageRole::Assistant,
            is_final: None,
        }));
        fmt.handle_event(&make_event(AgentEventType::TextMessageContent {
            message_id: "m1".into(),
            step_id: "s1".into(),
            delta: "Hello world!".into(),
            stripped_content: None,
        }));

        match fmt.surface_renderer().take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("Hello world!"));
            }
            other => panic!("Expected RichText, got {:?}", other),
        }
    }

    #[test]
    fn probe_calls_hidden() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "load_skill".into(),
            input: serde_json::json!({"skill_name": "?"}),
        }));

        match fmt.surface_renderer().take_output() {
            RendererOutput::None => {} // correct — probe calls should not produce output
            other => panic!("Expected None for probe call, got {:?}", other),
        }
    }

    #[test]
    fn long_message_splits() {
        let long_text = "a".repeat(5000);
        let chunks = split_message(&long_text, 4000);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 4000);
        }
    }
}
