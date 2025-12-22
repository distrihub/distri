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
}

/// A portable event printer that mirrors the CLI printer behavior but has no CLI-specific
/// dependencies. Can be embedded by CLI, servers, or other clients.
pub struct EventPrinter {
    state: ChatState,
}

impl EventPrinter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
        }
    }

    pub async fn handle_event(&mut self, event: &AgentEvent) {
        match &event.event {
            AgentEventType::PlanStarted { initial_plan } => {
                self.state.is_planning = true;
                println!(
                    "{}🧠 Planning{}{}",
                    COLOR_BRIGHT_YELLOW,
                    if *initial_plan { "…" } else { " (refine)…" },
                    COLOR_RESET
                );
            }
            AgentEventType::PlanFinished { total_steps } => {
                self.state.is_planning = false;
                println!(
                    "{}✅ Plan ready ({} steps){}",
                    COLOR_GREEN, total_steps, COLOR_RESET
                );
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
                println!(
                    "{}→ Starting step {}{}",
                    COLOR_CYAN,
                    step_index + 1,
                    COLOR_RESET
                );
            }
            AgentEventType::StepCompleted { step_id, success } => {
                if let Some(step) = self.state.steps.get_mut(step_id) {
                    step.status = if *success {
                        "done".into()
                    } else {
                        "error".into()
                    };
                    step.end_time = Some(Instant::now());
                    let elapsed = step
                        .start_time
                        .map(|s| s.elapsed().as_millis())
                        .unwrap_or(0);
                    println!(
                        "{}{} Step {} ({}) [{}ms]{}",
                        if *success { COLOR_GREEN } else { COLOR_RED },
                        if *success { "✔" } else { "✖" },
                        step.index + 1,
                        step.title,
                        elapsed,
                        COLOR_RESET
                    );
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
            AgentEventType::RunFinished {
                success,
                total_steps,
                ..
            } => {
                let stamp = Local::now().format("%H:%M:%S").to_string();
                println!(
                    "{}{} [{}] run finished ({} steps, {}){}",
                    COLOR_GREEN,
                    stamp,
                    event.agent_id,
                    total_steps,
                    if *success { "ok" } else { "error" },
                    COLOR_RESET
                );
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

        let role_label = match role {
            MessageRole::Assistant => "assistant",
            MessageRole::User => "user",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        };
        print!("{}{}:{} ", COLOR_CYAN, role_label, COLOR_RESET);
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

    fn tool_start(&mut self, tool_call_id: &str, name: &str, input: &serde_json::Value) {
        println!(
            "{}⏺ {} ({}){}",
            COLOR_YELLOW,
            name,
            self.format_tool_input(input),
            COLOR_RESET
        );
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
            let elapsed = state
                .start_time
                .map(|s| s.elapsed().as_millis())
                .unwrap_or(0);
            println!(
                "{}{} completed in {}ms{}",
                if success { COLOR_GREEN } else { COLOR_RED },
                state.tool_name,
                elapsed,
                COLOR_RESET
            );
        }
    }

    fn handle_tool_calls(&mut self, tool_calls: &[distri_types::ToolCall], parent: Option<&str>) {
        if let Some(parent) = parent {
            println!(
                "{}Tool calls for message {}{}",
                COLOR_GRAY, parent, COLOR_RESET
            );
        }
        for call in tool_calls {
            println!(
                "{}• {} ({}){}",
                COLOR_GRAY,
                call.tool_name,
                self.format_tool_input(&call.input),
                COLOR_RESET
            );
        }
    }

    fn print_tool_result(&self, result: &ToolResponse) {
        if let Ok(json) = serde_json::to_string_pretty(&result.parts) {
            println!("{}Tool result{}:\n{}", COLOR_GRAY, COLOR_RESET, json);
        }
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
    let printer = Arc::new(Mutex::new(EventPrinter::new()));
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
                }
            }
        })
        .await
}
