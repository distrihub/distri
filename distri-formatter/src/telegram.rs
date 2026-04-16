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
    pub(crate) output: String,
    /// Pending media attachments.
    pub(crate) media: Vec<MediaAttachment>,
    /// Whether there is new output to flush.
    pub(crate) dirty: bool,
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
            artifact_path: None,
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

        // Track agent changes — emit a line when switching (except the first agent).
        let agent_changed = self
            .state
            .current_agent
            .as_ref()
            .map(|a| a != &event.agent_id)
            .unwrap_or(true);
        if agent_changed && !event.agent_id.is_empty() {
            if self.state.current_agent.is_some() {
                let line = format!(
                    "\n<b>Agent: {}</b>\n",
                    crate::telegram_html::escape_html(&event.agent_id)
                );
                self.renderer.render_text(&line);
            }
            self.state.current_agent = Some(event.agent_id.clone());
        }

        match &event.event {
            AgentEventType::PlanStarted { .. } => {
                self.state.is_planning = true;
                self.renderer.show_planning("Planning…");
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
                if !*success {
                    let idx = self
                        .state
                        .steps
                        .get(step_id)
                        .map(|s| s.index + 1)
                        .unwrap_or(0);
                    self.renderer
                        .render_text(&format!("\n<b>Step {idx} failed</b>\n"));
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
                    let call_text = crate::state::format_tool_call(tool_call_name, input);
                    let line = format!(
                        "\n<b>{}</b>\n",
                        crate::telegram_html::escape_html(&call_text)
                    );
                    self.renderer.render_text(&line);
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
                    // Render tool output as HTML
                    let html = crate::telegram_html::format_tool_result_html(result);
                    if !html.is_empty() {
                        self.renderer.render_text(&format!("{html}\n"));
                    }
                    // Extract any Part::Artifact — images become media attachments
                    for part in &result.parts {
                        if let distri_types::Part::Artifact(meta) = part {
                            let mime = meta
                                .content_type
                                .as_deref()
                                .unwrap_or("application/octet-stream");
                            if mime.starts_with("image/") && !meta.relative_path.is_empty() {
                                self.renderer.media.push(crate::MediaAttachment {
                                    data: Vec::new(),
                                    mime_type: mime.to_string(),
                                    filename: meta.original_filename.clone(),
                                    artifact_path: Some(meta.relative_path.clone()),
                                });
                                self.renderer.dirty = true;
                            } else {
                                // Non-image artifact: render as inline link/preview
                                let name = meta
                                    .original_filename
                                    .clone()
                                    .unwrap_or_else(|| meta.file_id.clone());
                                self.renderer.render_text(&format!(
                                    "<i>Artifact: {} ({})</i>\n",
                                    crate::telegram_html::escape_html(&name),
                                    crate::telegram_html::escape_html(mime)
                                ));
                            }
                        }
                    }
                }
            }
            AgentEventType::ToolCalls { .. } => {}
            AgentEventType::RunFinished { success, .. } => {
                if !*success {
                    self.renderer
                        .render_text("\n<b>Run completed with errors</b>\n");
                }
            }
            AgentEventType::RunError { message, code, .. } => {
                let code_str = code
                    .as_ref()
                    .map(|c| format!(" [{}]", crate::telegram_html::escape_html(c)))
                    .unwrap_or_default();
                self.renderer.render_text(&format!(
                    "\n<b>Error{code_str}:</b> {}\n",
                    crate::telegram_html::escape_html(message)
                ));
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                let reason_str = reason
                    .as_deref()
                    .map(|r| format!(" ({})", crate::telegram_html::escape_html(r)))
                    .unwrap_or_default();
                self.renderer.render_text(&format!(
                    "\n<b>Transferring: {} → {}{}</b>\n",
                    crate::telegram_html::escape_html(from_agent),
                    crate::telegram_html::escape_html(to_agent),
                    reason_str
                ));
            }
            AgentEventType::TodosUpdated {
                formatted_todos, ..
            } => {
                if !formatted_todos.is_empty() {
                    self.renderer.render_text(&format!(
                        "\n<b>Plan updated:</b>\n<pre>{}</pre>\n",
                        crate::telegram_html::escape_html(formatted_todos)
                    ));
                }
            }
            AgentEventType::ContextCompaction {
                tier,
                tokens_before,
                tokens_after,
                ..
            } => {
                let tier_label = match tier {
                    distri_types::CompactionTier::Trim => "Trim",
                    distri_types::CompactionTier::Summarize => "Summarize",
                    distri_types::CompactionTier::Reset => "Reset",
                };
                let pct = if *tokens_before > 0 {
                    (1.0 - *tokens_after as f64 / *tokens_before as f64) * 100.0
                } else {
                    0.0
                };
                self.renderer.render_text(&format!(
                    "\n<i>Context {tier_label}: {tokens_before} → {tokens_after} tokens ({pct:.0}% reduction)</i>\n"
                ));
            }
            AgentEventType::LiveView { url, title, .. } => {
                let label = title.as_deref().unwrap_or("Live view");
                // Only allow http(s):// URLs with no characters that would break out of
                // the href attribute. Otherwise fall back to a title-only line.
                let is_safe_url = (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.contains(['"', '<', '>', '\'', ' ', '\n', '\r', '\t']);
                if is_safe_url {
                    self.renderer.render_text(&format!(
                        "\n<b>{}</b>\n<a href=\"{}\">{}</a>\n",
                        crate::telegram_html::escape_html(label),
                        crate::telegram_html::escape_html(url),
                        crate::telegram_html::escape_html(url),
                    ));
                } else {
                    self.renderer.render_text(&format!(
                        "\n<b>{}</b>\n<i>(unsafe URL not rendered)</i>\n",
                        crate::telegram_html::escape_html(label),
                    ));
                }
            }
            // Explicit no-ops for events we intentionally don't render in chat.
            AgentEventType::DiagnosticLog { .. }
            | AgentEventType::ReflectStarted {}
            | AgentEventType::ReflectFinished { .. }
            | AgentEventType::PlanPruned { .. }
            | AgentEventType::ContextBudgetUpdate { .. }
            | AgentEventType::RunStarted {}
            | AgentEventType::BrowserSessionStarted { .. }
            | AgentEventType::InlineHookRequested { .. } => {}
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
                // New behavior: tool calls render as <b>name(args)</b>, not status text
                assert!(text.contains("search"), "should mention tool name: got {text}");
                assert!(text.contains("rust async"), "should mention query: got {text}");
            }
            other => panic!("Expected RichText, got {:?}", other),
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

    #[test]
    fn tool_call_renders_formatted_line() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "Bash".into(),
            input: serde_json::json!({"command": "echo hello"}),
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("Bash"), "should show tool name: got {text}");
                assert!(text.contains("echo hello"), "should show command: got {text}");
            }
            other => panic!("Expected RichText, got {:?}", other),
        }
    }

    #[test]
    fn tool_result_renders_output() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::ToolExecutionStart {
            step_id: "s1".into(),
            tool_call_id: "tc1".into(),
            tool_call_name: "Bash".into(),
            input: serde_json::json!({"command": "echo hello"}),
        }));
        let _ = fmt.take_output();

        fmt.handle_event(&make_event(AgentEventType::ToolResults {
            step_id: "s1".into(),
            parent_message_id: None,
            results: vec![distri_types::ToolResponse::direct(
                "tc1".into(),
                "Bash".into(),
                serde_json::json!({"stdout": "hello\n", "stderr": "", "exit_code": 0}),
            )],
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("hello"), "tool result should render: got {text}");
            }
            other => panic!("Expected RichText with tool result, got {:?}", other),
        }
    }

    #[test]
    fn step_failure_renders_error() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::StepStarted {
            step_id: "s1".into(),
            step_index: 0,
        }));
        fmt.handle_event(&make_event(AgentEventType::StepCompleted {
            step_id: "s1".into(),
            success: false,
            context_budget: None,
            usage: None,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(
                    text.to_lowercase().contains("failed"),
                    "should show step failure: got {text}"
                );
            }
            other => panic!("Expected RichText with failure notice, got {:?}", other),
        }
    }

    #[test]
    fn run_error_renders_with_code() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::RunError {
            message: "quota exceeded".into(),
            code: Some("rate_limit".into()),
            usage: None,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("quota exceeded"), "should show message");
                assert!(text.contains("rate_limit"), "should show error code");
            }
            other => panic!("Expected RichText with error, got {:?}", other),
        }
    }

    #[test]
    fn todos_updated_renders() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::TodosUpdated {
            formatted_todos: "1. [x] Setup\n2. [ ] Build".into(),
            action: "write_todos".into(),
            todo_count: 2,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("Setup"));
                assert!(text.contains("Build"));
            }
            other => panic!("Expected RichText with todos, got {:?}", other),
        }
    }

    #[test]
    fn live_view_renders_with_title_and_link() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::LiveView {
            view_id: "v1".into(),
            url: "https://browsr.example.com/viewer/abc".into(),
            title: Some("Live browser".into()),
            display_mode: Some("inline".into()),
            width: None,
            height: None,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(text.contains("Live browser"), "should show title");
                assert!(
                    text.contains("browsr.example.com"),
                    "should include URL: got {text}"
                );
            }
            other => panic!("Expected RichText with live view, got {:?}", other),
        }
    }

    #[test]
    fn live_view_rejects_unsafe_urls() {
        // A URL containing a quote would break out of the href attribute.
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::LiveView {
            view_id: "v1".into(),
            url: "https://evil.example.com/\"><script>alert(1)</script>".into(),
            title: Some("Bad url".into()),
            display_mode: None,
            width: None,
            height: None,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(!text.contains("<script>"), "raw <script> must not leak: got {text}");
                assert!(!text.contains("onclick="), "no event handlers either");
                assert!(text.contains("Bad url"), "title should still render");
                // The href attribute must not be present at all, since we rejected the URL
                assert!(!text.contains("href="), "href should NOT be rendered for unsafe URL: got {text}");
            }
            other => panic!("Expected RichText, got {:?}", other),
        }
    }

    #[test]
    fn live_view_rejects_non_http_urls() {
        let mut fmt = TelegramFormatter::new();
        fmt.handle_event(&make_event(AgentEventType::LiveView {
            view_id: "v1".into(),
            url: "javascript:alert(1)".into(),
            title: None,
            display_mode: None,
            width: None,
            height: None,
        }));

        match fmt.take_output() {
            RendererOutput::RichText { text, .. } => {
                assert!(!text.contains("javascript:"), "javascript: URL must not appear: got {text}");
                assert!(!text.contains("href="), "no href for non-http URL");
            }
            other => panic!("Expected RichText, got {:?}", other),
        }
    }

    #[test]
    fn artifact_image_produces_media_attachment() {
        let mut fmt = TelegramFormatter::new();
        let meta = distri_types::FileMetadata {
            file_id: "chart.png".into(),
            relative_path: "threads/t1/tasks/t2/content/chart.png".into(),
            size: 1024,
            content_type: Some("image/png".into()),
            original_filename: Some("chart.png".into()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            checksum: None,
            stats: None,
            preview: None,
        };

        fmt.handle_event(&make_event(AgentEventType::ToolResults {
            step_id: "s1".into(),
            parent_message_id: None,
            results: vec![distri_types::ToolResponse::from_parts(
                "tc1".into(),
                "save_artifact".into(),
                vec![distri_types::Part::Artifact(meta)],
            )],
        }));

        match fmt.take_output() {
            RendererOutput::RichText { media, .. } => {
                assert_eq!(media.len(), 1, "should have one media attachment");
                assert_eq!(
                    media[0].artifact_path.as_deref(),
                    Some("threads/t1/tasks/t2/content/chart.png")
                );
                assert_eq!(media[0].mime_type, "image/png");
                assert!(
                    media[0].data.is_empty(),
                    "data should be empty — gateway fetches bytes"
                );
            }
            other => panic!("Expected RichText with media, got {:?}", other),
        }
    }
}
