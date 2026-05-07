pub mod colors;
pub mod extract;
pub mod renderers;
pub mod state;
pub mod status;
pub mod text;

// Per-platform surface renderers (TelegramFormatter, WhatsAppFormatter,
// telegram_html) moved to distri-gateway. distri-formatter keeps only the
// trait + shared state + default text renderer.

use distri_types::{AgentEvent, ToolResponse};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Parse mode for rich-text output (maps to platform APIs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParseMode {
    /// Telegram Markdown
    Markdown,
    /// Telegram MarkdownV2
    MarkdownV2,
    /// HTML
    Html,
    /// Plain text (no parsing)
    Plain,
}

/// A media attachment (image, document, etc.) produced by a renderer.
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    /// Raw bytes. Empty when `artifact_path` is set (gateway fetches bytes).
    pub data: Vec<u8>,
    pub mime_type: String,
    pub filename: Option<String>,
    /// If set, channels should fetch bytes from the artifact store at this
    /// path instead of using `data`. Enables lazy byte-loading for artifacts.
    pub artifact_path: Option<String>,
}

/// What a surface renderer produces after handling events.
#[derive(Debug)]
pub enum RendererOutput {
    /// Terminal renderer already printed to stdout — nothing to collect.
    Terminal,
    /// No output ready yet.
    None,
    /// Plain text fallback.
    Text(String),
    /// Formatted output for channels (Telegram, WhatsApp, etc.).
    ///
    /// When `is_streaming_text` is true, the content is accumulated LLM text
    /// that should be edited in place in the current channel message (growing
    /// a single message as more text streams in). When false, it's a
    /// structural event (tool call, tool result, error, handover) that should
    /// be sent as a fresh message and end any in-progress streaming edit.
    RichText {
        text: String,
        parse_mode: ParseMode,
        media: Vec<MediaAttachment>,
        #[doc(hidden)]
        is_streaming_text: bool,
    },
    /// Split messages (e.g. Telegram 4K limit).
    Chunks(Vec<RendererOutput>),
}

// ---------------------------------------------------------------------------
// Formatter trait (shared state machine)
// ---------------------------------------------------------------------------

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

    /// Take any pending renderer output.
    fn take_output(&mut self) -> RendererOutput;

    /// Clear accumulated output so it can be replaced (e.g. by the final message).
    fn clear_content(&mut self);

    /// Handle the final A2A message (from `final` tool or agent completion).
    /// Called when the stream item carries `item.message` with an assistant
    /// message — same as what the CLI does in `print_stream_verbose`.
    ///
    /// The final message replaces any previously accumulated content because
    /// the `final` tool's output IS the actual answer — earlier streamed text
    /// was the agent thinking aloud before calling tools.
    fn handle_final_message(&mut self, message: &distri_types::Message) {
        if message.role != distri_types::MessageRole::Assistant {
            return;
        }
        if let Some(text) = message.as_text()
            && !text.is_empty()
        {
            // Replace accumulated content with the final answer.
            self.clear_content();
            self.handle_event(&distri_types::AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: String::new(),
                run_id: String::new(),
                event: distri_types::AgentEventType::TextMessageContent {
                    message_id: message.id.clone(),
                    step_id: String::new(),
                    delta: text,
                    stripped_content: None,
                },
                task_id: String::new(),
                parent_task_id: None,
                agent_id: String::new(),
                user_id: None,
                identifier_id: None,
                workspace_id: None,
                channel_id: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// SurfaceRenderer trait (per-surface rendering)
// ---------------------------------------------------------------------------

/// Surface-specific rendering — each channel/surface implements this.
///
/// The `Formatter` drives state transitions and calls these methods to produce
/// output appropriate for the target surface (Terminal, Telegram, WhatsApp, etc.).
pub trait SurfaceRenderer: Send + Sync {
    // --- Text content ---
    fn render_text(&mut self, content: &str);
    fn render_markdown(&mut self, md: &str);
    fn render_code_block(&mut self, code: &str, lang: Option<&str>);
    fn render_diff(&mut self, diff: &str);

    // --- Tool progress ---
    fn render_tool_start(&mut self, name: &str, input: &serde_json::Value, status_text: &str);
    fn render_tool_result(&mut self, name: &str, result: &ToolResponse, verbose: bool);
    fn render_status_update(&mut self, text: &str);

    // --- Media ---
    fn render_image(&mut self, data: &[u8], mime: &str);

    // --- Planning/loading ---
    fn show_planning(&mut self, phrase: &str);
    fn clear_planning(&mut self);

    // --- Agent handover ---
    fn render_agent_transfer(&mut self, from: &str, to: &str, reason: Option<&str>);

    // --- Capabilities ---
    fn supports_images(&self) -> bool;
    fn supports_rich_text(&self) -> bool;
    fn max_message_length(&self) -> Option<usize>;

    // --- Structured content ---
    /// Structured content rendering hook for downstream crates.
    ///
    /// Receives the tool name (e.g. `render_card`, `ask_follow_up`) and its
    /// JSON input. Downstream crates override this to produce channel-specific
    /// rich output (inline keyboards, interactive messages, etc.).
    ///
    /// Returns `RendererOutput::None` by default — plain-text surfaces ignore
    /// structured content and let the agent's text response serve as fallback.
    fn render_structured(&mut self, _tool_name: &str, _data: &serde_json::Value) -> RendererOutput {
        RendererOutput::None
    }

    // --- Output ---
    /// Take any pending output. Returns `RendererOutput::None` if nothing is ready.
    fn take_output(&mut self) -> RendererOutput;
}
