use std::{collections::HashMap, sync::Arc, time::Instant};

use anyhow::Context;
use base64::{Engine as _, engine::general_purpose};
use chrono::Local;
use distri_a2a::MessageSendParams;
use distri_types::{AgentEvent, AgentEventType, MessageRole, ToolResponse};
use image::DynamicImage;
use tokio::sync::Mutex;
use viuer::Config;

use crate::client_stream::{AgentStreamClient, StreamError, StreamItem};

pub const COLOR_RESET: &str = "\x1b[0m";
pub const COLOR_RED: &str = "\x1b[31m";
pub const COLOR_GREEN: &str = "\x1b[32m";
pub const COLOR_YELLOW: &str = "\x1b[33m";
pub const COLOR_CYAN: &str = "\x1b[36m";
pub const COLOR_GRAY: &str = "\x1b[90m";
pub const COLOR_BRIGHT_CYAN: &str = "\x1b[96m";
pub const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum ToolCallStatus {
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ToolCallState {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub status: ToolCallStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StepState {
    pub id: String,
    pub title: String,
    pub index: usize,
    pub status: String,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MessageState {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub is_streaming: bool,
    pub is_complete: bool,
    pub step_id: Option<String>,
}

#[derive(Debug, Default)]
struct ChatState {
    messages: HashMap<String, MessageState>,
    steps: HashMap<String, StepState>,
    tool_calls: HashMap<String, ToolCallState>,
    current_message_id: Option<String>,
    is_planning: bool,
    printed_header: bool,
    current_agent: Option<String>,
}

pub struct EventPrinter {
    state: ChatState,
    verbose: bool,
    show_tools: bool,
    agent_display_name: Option<String>,
}

impl EventPrinter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
            verbose: false,
            show_tools: true,
            agent_display_name: None,
        }
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn with_agent_name(mut self, name: String) -> Self {
        self.agent_display_name = Some(name);
        self
    }

    /// Toggle tool output visibility. Returns the new state.
    pub fn toggle_show_tools(&mut self) -> bool {
        self.show_tools = !self.show_tools;
        self.show_tools
    }

    pub fn show_tools(&self) -> bool {
        self.show_tools
    }

    /// If "Planning…" is showing on the current line, clear it before printing anything else.
    fn clear_planning_line(&mut self) {
        if self.state.is_planning {
            print!("\r\x1b[2K");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            self.state.is_planning = false;
        }
    }

    pub async fn handle_event(&mut self, event: &AgentEvent) {
        // Track first event (no header printed — internal IDs aren't useful)
        if !self.state.printed_header {
            self.state.printed_header = true;
        }

        // Track agent changes and display them
        let agent_changed = self
            .state
            .current_agent
            .as_ref()
            .map(|a| a != &event.agent_id)
            .unwrap_or(true);
        if agent_changed && !event.agent_id.is_empty() {
            // Only print if it's not the first agent (header already shows it)
            if self.state.current_agent.is_some() {
                self.clear_planning_line();
                println!(
                    "{}⇢ Agent: {}{}",
                    COLOR_BRIGHT_CYAN, event.agent_id, COLOR_RESET
                );
            }
            self.state.current_agent = Some(event.agent_id.clone());
        }

        // Clear the transient "Planning…" line before any other event prints output
        match &event.event {
            AgentEventType::PlanStarted { .. } | AgentEventType::PlanFinished { .. } => {}
            _ => self.clear_planning_line(),
        }

        match &event.event {
            AgentEventType::PlanStarted { initial_plan } => {
                self.state.is_planning = true;
                print!(
                    "\r\x1b[2K{}🧠 Planning{}{}",
                    COLOR_BRIGHT_YELLOW,
                    if *initial_plan { "…" } else { " (refine)…" },
                    COLOR_RESET
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            AgentEventType::PlanFinished { .. } => {
                // Clear the planning line and reset flag
                self.clear_planning_line();
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
                        start_time: Some(Instant::now()),
                        end_time: None,
                    },
                );
                // Suppressed — individual tool calls show progress
            }
            AgentEventType::StepCompleted { step_id, success } => {
                if let Some(step) = self.state.steps.get_mut(step_id) {
                    step.status = if *success {
                        "done".into()
                    } else {
                        "error".into()
                    };
                    step.end_time = Some(Instant::now());
                    // Only show on failure
                    if !success {
                        println!(
                            "{}✖ Step {} failed{}",
                            COLOR_RED,
                            step.index + 1,
                            COLOR_RESET
                        );
                    }
                }
            }
            AgentEventType::TextMessageStart {
                message_id, role, ..
            } => {
                self.start_message(message_id, role);
            }
            AgentEventType::TextMessageContent {
                message_id, delta, ..
            } => {
                self.append_message(message_id, delta);
            }
            AgentEventType::TextMessageEnd { message_id, .. } => {
                self.finish_message(message_id);
            }
            AgentEventType::ToolExecutionStart {
                tool_call_id,
                tool_call_name,
                input,
                ..
            } => {
                self.tool_start(tool_call_id, tool_call_name, input);
            }
            AgentEventType::ToolExecutionEnd {
                tool_call_id,
                success,
                ..
            } => {
                self.tool_end(tool_call_id, *success);
            }
            AgentEventType::ToolResults { results, .. } => {
                for result in results {
                    self.print_tool_result(result);
                }
            }
            AgentEventType::ToolCalls {
                tool_calls,
                parent_message_id,
                ..
            } => {
                self.handle_tool_calls(tool_calls, parent_message_id.as_deref());
            }
            AgentEventType::RunFinished { success, .. } => {
                if !success {
                    println!(
                        "{}Run completed with errors{}",
                        COLOR_RED, COLOR_RESET
                    );
                }
            }
            AgentEventType::RunError { message, code } => {
                let stamp = Local::now().format("%H:%M:%S").to_string();
                println!(
                    "{}{} [{}] run failed: {} ({:?}){}",
                    COLOR_RED, stamp, event.agent_id, message, code, COLOR_RESET
                );
            }
            AgentEventType::InlineHookRequested { request } => {
                println!(
                    "{}Awaiting inline hook {} for {} with input {}{}",
                    COLOR_BRIGHT_CYAN,
                    request.hook_id,
                    request.hook,
                    self.format_tool_input(&serde_json::json!(request.message)),
                    COLOR_RESET
                );
            }
            AgentEventType::TodosUpdated {
                formatted_todos, ..
            } => {
                println!(
                    "{}Todos updated{}:\n{}",
                    COLOR_GRAY, COLOR_RESET, formatted_todos
                );
            }
            AgentEventType::BrowserScreenshot { .. } => {
                // Render the screenshot if present in metadata
                if let AgentEventType::BrowserScreenshot {
                    image,
                    format,
                    filename,
                    size,
                    timestamp_ms,
                } = &event.event
                {
                    if let Err(err) = self.print_browser_image(
                        image,
                        format.as_deref(),
                        filename.as_deref(),
                        *size,
                        *timestamp_ms,
                    ) {
                        println!(
                            "{}📸 Browser screenshot (render failed: {}){}",
                            COLOR_GRAY, err, COLOR_RESET
                        );
                    }
                }
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
                println!(
                    "{}⇢ Transferring: {} → {}{}{}",
                    COLOR_BRIGHT_CYAN, from_agent, to_agent, reason_str, COLOR_RESET
                );
            }
            _ => {}
        }
    }

    fn start_message(&mut self, message_id: &str, role: &MessageRole) {
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

        match role {
            MessageRole::Assistant => {
                let name = self
                    .agent_display_name
                    .as_deref()
                    .or(self.state.current_agent.as_deref())
                    .unwrap_or("assistant");
                print!("{}◆ {}:{} ", COLOR_CYAN, name, COLOR_RESET);
            }
            _ => {
                let role_label = match role {
                    MessageRole::User => "user",
                    MessageRole::System => "system",
                    MessageRole::Tool => "tool",
                    MessageRole::Developer => "developer",
                    MessageRole::Assistant => unreachable!(),
                };
                print!("{}{}:{} ", COLOR_CYAN, role_label, COLOR_RESET);
            }
        }
    }

    fn append_message(&mut self, message_id: &str, delta: &str) {
        if let Some(msg) = self.state.messages.get_mut(message_id) {
            msg.content.push_str(delta);
            print!("{delta}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }

    fn finish_message(&mut self, message_id: &str) {
        if let Some(msg) = self.state.messages.get_mut(message_id) {
            msg.is_streaming = false;
            msg.is_complete = true;
            println!();
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

    /// Returns true if this tool call looks like an internal discovery/probe call
    /// that shouldn't be shown to the user.
    fn is_probe_call(name: &str, input: &serde_json::Value) -> bool {
        match name {
            "load_skill" => {
                let skill = input
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                skill == "?" || skill.is_empty()
            }
            "api_request" => {
                // Probe requests (discovery GETs to nonexistent endpoints)
                let method = input
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                method == "GET"
                    && (path.ends_with("/v1/agents")
                        || path.ends_with("/v1/connections")
                        || path.ends_with("/v1/skills"))
            }
            _ => false,
        }
    }

    fn tool_start(&mut self, tool_call_id: &str, name: &str, input: &serde_json::Value) {
        if self.show_tools && !Self::is_probe_call(name, input) {
            println!(
                "{}⏺ {}{}",
                COLOR_YELLOW,
                Self::format_tool_call(name, input),
                COLOR_RESET
            );
        }
        self.state.tool_calls.insert(
            tool_call_id.to_string(),
            ToolCallState {
                tool_call_id: tool_call_id.to_string(),
                tool_name: name.to_string(),
                input: input.clone(),
                status: ToolCallStatus::Running,
                result: None,
                error: None,
                start_time: Some(Instant::now()),
                end_time: None,
            },
        );
    }

    fn tool_end(&mut self, tool_call_id: &str, success: bool) {
        if let Some(state) = self.state.tool_calls.get_mut(tool_call_id) {
            state.status = if success {
                ToolCallStatus::Completed
            } else {
                ToolCallStatus::Error
            };
            state.end_time = Some(Instant::now());
            // No output — the ⎿ result line is sufficient
        }
    }

    fn handle_tool_calls(
        &mut self,
        _tool_calls: &[distri_types::ToolCall],
        _parent: Option<&str>,
    ) {
        // Suppressed — individual ToolExecutionStart events show each call
    }

    fn print_tool_result(&self, result: &ToolResponse) {
        if !self.show_tools {
            return;
        }
        crate::renderers::render_tool_output(result, self.verbose);
    }

    fn print_browser_image(
        &self,
        image_data: &str,
        format: Option<&str>,
        filename: Option<&str>,
        size: Option<u64>,
        timestamp_ms: Option<i64>,
    ) -> Result<(), anyhow::Error> {
        let snapshot = Self::decode_browser_image(image_data)?;
        let cols = 100u16;
        let mut width_cells = cols.saturating_sub(4);
        if width_cells == 0 {
            width_cells = cols;
        }
        if width_cells == 0 {
            width_cells = 80;
        }
        width_cells = width_cells.min(160);

        let preview_width = width_cells.min(80);
        let config = Config {
            width: Some(u32::from(preview_width)),
            ..Default::default()
        };

        viuer::print(&snapshot, &config)
            .map_err(|err| anyhow::anyhow!("failed to display browser screenshot: {}", err))?;

        if let Some(meta) = Self::format_browser_metadata(format, filename, size, timestamp_ms) {
            println!("{}", meta);
        }

        println!();
        Ok(())
    }

    fn decode_browser_image(encoded_image: &str) -> Result<DynamicImage, anyhow::Error> {
        let trimmed = encoded_image.trim();
        let payload = if let Some(idx) = trimmed.find("base64,") {
            &trimmed[idx + "base64,".len()..]
        } else if let Some(idx) = trimmed.find(',') {
            &trimmed[idx + 1..]
        } else {
            trimmed
        };

        let sanitized: String = payload.split_whitespace().collect();

        let decoded = general_purpose::STANDARD
            .decode(sanitized.as_bytes())
            .context("failed to decode browser screenshot payload")?;

        image::load_from_memory(&decoded).map_err(|e| anyhow::anyhow!(e))
    }

    fn format_browser_metadata(
        format: Option<&str>,
        filename: Option<&str>,
        size: Option<u64>,
        timestamp_ms: Option<i64>,
    ) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(f) = format {
            parts.push(format!("format={}", f));
        }
        if let Some(name) = filename {
            parts.push(format!("file={}", name));
        }
        if let Some(bytes) = size {
            parts.push(format!("size={}B", bytes));
        }
        if let Some(ts) = timestamp_ms {
            parts.push(format!("ts={}", ts));
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!(
                "{}   {}{}",
                COLOR_GRAY,
                parts.join(" | "),
                COLOR_RESET
            ))
        }
    }

    fn format_tool_call(name: &str, input: &serde_json::Value) -> String {
        let str_field = |key: &str| {
            input
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string()
        };
        let truncate = |s: &str, max: usize| -> String {
            if s.len() > max {
                format!("{}…", &s[..max])
            } else {
                s.to_string()
            }
        };

        match name {
            "load_skill" => format!("load_skill(\"{}\")", str_field("skill_name")),
            "run_skill_script" => {
                let skill = str_field("skill_name");
                match input.get("step_index").and_then(|v| v.as_u64()) {
                    Some(s) => format!("run_skill_script(\"{}\", step={})", skill, s),
                    None => format!("run_skill_script(\"{}\")", skill),
                }
            }
            "create_skill" | "delete_skill" => {
                let skill = input
                    .get("name")
                    .or(input.get("skill_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("{}(\"{}\")", name, skill)
            }
            "browsr_scrape" | "browsr_crawl" => {
                format!("{}(\"{}\")", name, truncate(&str_field("url"), 60))
            }
            "browsr_browser" | "browser_step" => {
                let action = str_field("action");
                match input.get("url").and_then(|v| v.as_str()) {
                    Some(u) => format!("{}({} \"{}\")", name, action, truncate(u, 50)),
                    None => format!("{}({})", name, action),
                }
            }
            "execute_shell" => {
                format!("execute_shell(\"{}\")", truncate(&str_field("command"), 60))
            }
            "start_shell" | "stop_shell" => format!("{}(…)", name),
            "search" => format!("search(\"{}\")", truncate(&str_field("query"), 60)),
            "write_to_storage" | "read_from_storage" => {
                let key = input
                    .get("key")
                    .or(input.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                format!("{}(\"{}\")", name, key)
            }
            "tool_search" => format!("tool_search(\"{}\")", truncate(&str_field("query"), 60)),
            "inject_connection_env" => {
                format!(
                    "inject_connection_env(\"{}\")",
                    str_field("provider_name")
                )
            }
            "transfer_to_agent" => {
                format!("transfer_to_agent(\"{}\")", str_field("agent_name"))
            }
            "final" | "reflect" | "console_log" => format!("{}(…)", name),
            _ => {
                let compact = serde_json::to_string(input).unwrap_or_else(|_| "…".into());
                let preview = truncate(&compact, 80);
                format!("{}({})", name, preview)
            }
        }
    }

    fn format_tool_input(&self, input: &serde_json::Value) -> String {
        if input.is_object() && input.as_object().map(|m| m.is_empty()).unwrap_or(false) {
            return "...".into();
        }
        serde_json::to_string(input).unwrap_or_else(|_| "...".into())
    }
}

/// Convenience helper that streams an agent and prints events to stdout.
pub async fn print_stream(
    client: &AgentStreamClient,
    agent_id: &str,
    params: MessageSendParams,
) -> Result<(), StreamError> {
    print_stream_verbose(client, agent_id, params, false, None, true).await
}

/// Convenience helper that streams an agent and prints events to stdout,
/// with optional verbose tool output.
pub async fn print_stream_verbose(
    client: &AgentStreamClient,
    agent_id: &str,
    params: MessageSendParams,
    verbose: bool,
    agent_display_name: Option<String>,
    show_tools: bool,
) -> Result<(), StreamError> {
    let mut printer = EventPrinter::new().with_verbose(verbose);
    if let Some(name) = agent_display_name {
        printer = printer.with_agent_name(name);
    }
    if !show_tools {
        printer.toggle_show_tools();
    }
    let printer = Arc::new(Mutex::new(printer));
    client
        .stream_agent(agent_id, params, {
            let printer = printer.clone();
            move |item: StreamItem| {
                let printer = printer.clone();
                async move {
                    if let Some(event) = item.agent_event.clone() {
                        let mut guard = printer.lock().await;
                        guard.handle_event(&event).await;
                    }
                    // Print the final assistant message text
                    if let Some(ref msg) = item.message {
                        if msg.role == distri_types::MessageRole::Assistant {
                            if let Some(text) = msg.as_text() {
                                if !text.is_empty() {
                                    println!("\n{}", text);
                                }
                            }
                        }
                    }
                }
            }
        })
        .await
}
