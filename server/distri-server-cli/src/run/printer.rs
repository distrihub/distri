use crate::tool_renderers::ToolRendererRegistry;
use anyhow::{anyhow, Context};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Local, Utc};
use crossterm::terminal;
use distri_core::agent::{AgentEventType, AgentOrchestrator, ExecutorContext, InvokeResult};
use distri_core::types::{Message, MessageRole, StandardDefinition};
use distri_types::configuration::DefinitionOverrides;
use image::{self, DynamicImage};
use serde_json::{self};
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::debug;
use viuer::Config;

pub const COLOR_RESET: &str = "\x1b[0m";
pub const COLOR_RED: &str = "\x1b[31m";
pub const COLOR_GREEN: &str = "\x1b[32m";
pub const COLOR_YELLOW: &str = "\x1b[33m";
pub const COLOR_CYAN: &str = "\x1b[36m";
pub const COLOR_WHITE: &str = "\x1b[37m";
pub const COLOR_BRIGHT_BLUE: &str = "\x1b[94m";
pub const COLOR_BRIGHT_GREEN: &str = "\x1b[92m";
pub const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";
pub const COLOR_BRIGHT_MAGENTA: &str = "\x1b[95m";
pub const COLOR_BRIGHT_CYAN: &str = "\x1b[96m";
pub const COLOR_GRAY: &str = "\x1b[90m";
pub const COLOR_BRIGHT_WHITE: &str = "\x1b[97m";
pub const COLOR_DISTRI_TEAL: &str = "\x1b[38;2;0;124;145m";

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStatus {
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone)]
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
pub struct StepState {
    pub id: String,
    pub title: String,
    pub index: usize,
    pub status: String,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct MessageState {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub is_streaming: bool,
    pub is_complete: bool,
    pub step_id: Option<String>,
}

#[derive(Debug)]
struct ChatState {
    messages: HashMap<String, MessageState>,
    steps: HashMap<String, StepState>,
    tool_calls: HashMap<String, ToolCallState>,
    current_run_id: Option<String>,
    current_message_id: Option<String>,
    is_streaming: bool,
    is_planning: bool,
    plan_start_time: Option<Instant>,
}

/// Format tool execution message like Claude Code
fn format_tool_execution_message(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "bash" | "shell" => {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                format!("Bash({})", command)
            } else {
                format!("{}({})", tool_name, input)
            }
        }
        "read" | "read_file" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                format!("Read({})", path)
            } else {
                format!("{}({})", tool_name, input)
            }
        }
        name if name.starts_with("fs_read") => {
            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                format!("Read({})", path)
            } else {
                format!("{}({})", tool_name, input)
            }
        }
        "edit" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                format!("Edit({})", path)
            } else {
                format!("{}({})", tool_name, input)
            }
        }
        _ => {
            // For other tools, show friendly name with key parameters
            let formatted = serde_json::to_string_pretty(input).unwrap_or_default();
            if !formatted.is_empty() || formatted != "{}" {
                format!("{}({})", tool_name, formatted.replace("\n", ""))
            } else {
                format!("{}(...)", tool_name)
            }
        }
    }
}

fn print_tool_completion(state: &ToolCallState, execution_time_ms: u64) -> bool {
    match state.status {
        ToolCallStatus::Completed => {
            let time_str = if execution_time_ms > 100 {
                format!(" ({:.1}s)", execution_time_ms as f64 / 1000.0)
            } else {
                String::new()
            };

            println!(
                "  ⎿  {} completed{}",
                get_friendly_tool_message(&state.tool_name, &state.input),
                time_str
            );

            if let Some(result) = &state.result {
                if let Ok(pretty) = serde_json::to_string_pretty(result) {
                    println!("      {}", pretty.replace("\n", "\n      "));
                }
            }
            true
        }
        ToolCallStatus::Error => {
            println!(
                "  ⎿  {} failed",
                get_friendly_tool_message(&state.tool_name, &state.input)
            );
            if let Some(error) = &state.error {
                println!("      Error: {}", error);
            }
            true
        }
        _ => false,
    }
}

fn get_friendly_tool_message(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "bash" | "shell" => {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                format!("Ran command: {}", command)
            } else {
                format!("Executed {}", tool_name)
            }
        }
        "read" | "read_file" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                format!("Read file: {}", path)
            } else {
                "Read file".to_string()
            }
        }
        "edit" => {
            if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                format!("Updated {}", path)
            } else {
                "Updated file".to_string()
            }
        }
        "search" => {
            if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
                format!("Searched: {}", query)
            } else {
                "Performed search".to_string()
            }
        }
        "todos" => {
            if let Some(action) = input.get("action").and_then(|v| v.as_str()) {
                match action {
                    "add" => {
                        if let Some(title) = input
                            .get("title")
                            .or_else(|| input.get("content"))
                            .and_then(|v| v.as_str())
                        {
                            format!("Added todo: {}", title)
                        } else {
                            "Added todo".to_string()
                        }
                    }
                    "update" => "Updated todo".to_string(),
                    "remove" => "Removed todo".to_string(),
                    "list" => "Listed todos".to_string(),
                    _ => format!("Managed todos ({})", action),
                }
            } else {
                "Managed todos".to_string()
            }
        }
        _ => format!("Executed {}", tool_name),
    }
}

fn todo_action_label(action: &str) -> &'static str {
    match action {
        "add" => "New Plan",
        "update" => "Updated Plan",
        "write" | "write_todos" => "Updated Plan",
        "remove" => "Plan Updated",
        "clear" => "Plan Cleared",
        "list" => "Plan Snapshot",
        _ => "Plan Update",
    }
}

impl ChatState {
    fn new() -> Self {
        Self {
            messages: HashMap::new(),
            steps: HashMap::new(),
            tool_calls: HashMap::new(),
            current_run_id: None,
            current_message_id: None,
            is_streaming: false,
            is_planning: false,
            plan_start_time: None,
        }
    }

    fn get_current_message_mut(&mut self) -> Option<&mut MessageState> {
        let id = self.current_message_id.clone();
        id.and_then(move |id| self.messages.get_mut(&id))
    }

    fn add_or_update_message(&mut self, id: String, role: MessageRole, step_id: Option<String>) {
        let message = MessageState {
            id: id.clone(),
            role,
            content: String::new(),
            is_streaming: true,
            is_complete: false,
            step_id,
        };
        self.messages.insert(id.clone(), message);
        self.current_message_id = Some(id);
    }

    fn append_to_current_message(&mut self, delta: &str) {
        if let Some(message) = self.get_current_message_mut() {
            message.content.push_str(delta);
        }
    }

    fn complete_current_message(&mut self) {
        if let Some(message) = self.get_current_message_mut() {
            message.is_streaming = false;
            message.is_complete = true;
        }
    }

    fn update_tool_call_status(
        &mut self,
        tool_call_id: &str,
        status: ToolCallStatus,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if let Some(tool_call) = self.tool_calls.get_mut(tool_call_id) {
            tool_call.status = status;
            tool_call.result = result;
            tool_call.error = error;
            tool_call.end_time = Some(Instant::now());
        }
    }
}

pub struct EventPrinter {
    state: ChatState,
    context: Arc<ExecutorContext>,
    max_iterations: usize,
    agent_definition: Option<StandardDefinition>,
    last_output_was_newline: bool,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
}

impl EventPrinter {
    pub fn new_with_context(
        context: Arc<ExecutorContext>,
        max_iterations: usize,
        tool_renderers: Option<Arc<ToolRendererRegistry>>,
    ) -> Self {
        Self {
            state: ChatState::new(),
            context,
            max_iterations,
            agent_definition: None,
            last_output_was_newline: true,
            tool_renderers,
        }
    }

    /// Set the agent definition for displaying strategy details
    pub fn set_agent_definition(&mut self, agent_def: StandardDefinition) {
        self.agent_definition = Some(agent_def);
    }

    fn ensure_newline(&mut self) {
        if !self.last_output_was_newline {
            println!();
            self.last_output_was_newline = true;
        }
    }

    fn print_with_tracking(&mut self, content: &str) {
        print!("{}", content);
        self.last_output_was_newline = content.ends_with('\n');
        io::stdout().flush().unwrap();
    }

    fn println_with_tracking(&mut self, content: &str) {
        println!("{}", content);
        self.last_output_was_newline = true;
    }

    /// Print current usage and iteration information
    async fn print_usage_info(&self) -> Result<(), anyhow::Error> {
        let usage = self.context.get_usage().await;
        let iteration_info = self.context.get_iteration_info(self.max_iterations).await;

        // Calculate context size on demand for current stats
        let context_size = self.context.calculate_context_size().await?;

        print!(
            "{}[Usage: {} tokens ({} in, {} out) | Context: ~{} tokens ({} chars) | Iteration: {}]{} ",
            COLOR_GRAY,
            usage.tokens,
            usage.input_tokens,
            usage.output_tokens,
            context_size.total_estimated_tokens,
            context_size.total_chars,
            iteration_info,
            COLOR_RESET
        );
        io::stdout().flush().unwrap();
        Ok(())
    }

    /// Print detailed context size summary
    async fn print_context_size_summary(&self) -> Result<(), anyhow::Error> {
        let context_size = self.context.calculate_context_size().await?;

        println!("   📋 Context Size Summary:");
        println!(
            "      Messages: {} messages (~{} tokens, {} chars)",
            context_size.message_count,
            context_size.message_estimated_tokens,
            context_size.message_chars
        );

        if context_size.execution_history_count > 0 {
            println!(
                "      Execution History: {} entries (~{} tokens, {} chars)",
                context_size.execution_history_count,
                context_size.execution_history_estimated_tokens,
                context_size.execution_history_chars
            );
        }

        if context_size.scratchpad_chars > 0 {
            println!(
                "      Scratchpad: ~{} tokens ({} chars)",
                context_size.scratchpad_estimated_tokens, context_size.scratchpad_chars
            );
        }

        println!(
            "      {}Total Context: ~{} tokens ({} chars){}",
            COLOR_BRIGHT_CYAN,
            context_size.total_estimated_tokens,
            context_size.total_chars,
            COLOR_RESET
        );

        // Display per-agent breakdown if there are multiple agents
        if context_size.agent_breakdown.len() > 1 {
            println!("      Agent Breakdown:");
            for (agent_id, stats) in &context_size.agent_breakdown {
                println!(
                    "        {}: {} tasks, ~{} tokens ({} exec entries)",
                    agent_id,
                    stats.task_count,
                    stats.execution_history_estimated_tokens + stats.scratchpad_estimated_tokens,
                    stats.execution_history_count
                );
            }
        }
        Ok(())
    }

    /// Handle tool call execution start with Claude Code style formatting
    fn handle_tool_execution_start(
        &mut self,
        tool_call_name: &str,
        tool_call_id: &str,
        input: &serde_json::Value,
    ) {
        self.ensure_newline();

        // Create tool call state
        let tool_state = ToolCallState {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_call_name.to_string(),
            input: input.clone(),
            status: ToolCallStatus::Running,
            result: None,
            error: None,
            start_time: Some(Instant::now()),
            end_time: None,
        };

        self.state
            .tool_calls
            .insert(tool_call_id.to_string(), tool_state);

        let mut rendered = false;
        if let Some(registry) = &self.tool_renderers {
            let outputs = registry.handle_tool_start(tool_call_id, tool_call_name, input);
            if !outputs.is_empty() {
                for output in outputs {
                    self.println_with_tracking(&output);
                }
                rendered = true;
            }
        }

        if !rendered {
            let formatted_message = format_tool_execution_message(tool_call_name, input);
            self.println_with_tracking(&format!(
                "{}⏺ {}{}",
                COLOR_YELLOW, formatted_message, COLOR_RESET
            ));
        }
    }

    /// Handle tool call completion
    fn handle_tool_execution_end(&mut self, tool_call_id: &str) {
        if let Some(tool_state) = self.state.tool_calls.get(tool_call_id).cloned() {
            let execution_time = tool_state
                .start_time
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0);

            let mut rendered = false;
            if let Some(registry) = &self.tool_renderers {
                let outputs = registry.handle_tool_end(
                    tool_call_id,
                    &tool_state.tool_name,
                    &tool_state.input,
                    tool_state.result.as_ref(),
                    matches!(tool_state.status, ToolCallStatus::Completed),
                );
                if !outputs.is_empty() {
                    rendered = true;
                    for output in outputs {
                        if !output.trim().is_empty() {
                            self.println_with_tracking(&output);
                        }
                    }
                }
            }

            if !rendered {
                print_tool_completion(&tool_state, execution_time);
            }
        }

        // Remove completed tool call
        self.state.tool_calls.remove(tool_call_id);
    }

    pub async fn handle_event(
        &mut self,
        event: distri_core::agent::AgentEvent,
    ) -> Result<(), anyhow::Error> {
        match event.event {
            AgentEventType::RunStarted {} => {
                self.state.current_run_id = Some(event.run_id.clone());
                self.state.is_streaming = true;

                // Clean, minimal start message like Claude Code
                if self.context.verbose {
                    self.println_with_tracking(&format!(
                        "{}Starting agent: {}{}",
                        COLOR_GRAY, event.agent_id, COLOR_RESET
                    ));
                    self.println_with_tracking("");
                }
            }
            AgentEventType::PlanStarted { initial_plan: _ } => {
                self.state.is_planning = true;
                self.state.plan_start_time = Some(Instant::now());

                // Show thinking indicator like Claude Code
                // self.print_with_tracking(&format!("{}⏺ Thinking...{}", COLOR_YELLOW, COLOR_RESET));
            }
            AgentEventType::PlanFinished { total_steps: _ } => {
                if self.state.is_planning {
                    // Clear the thinking line and show completion
                    print!("\r");

                    let duration_text = if let Some(start_time) = self.state.plan_start_time {
                        let duration = start_time.elapsed();
                        let seconds = duration.as_secs_f64();
                        format!("{:.1} seconds", seconds)
                    } else {
                        "a moment".to_string()
                    };

                    self.println_with_tracking(&format!(
                        "{}⏺ Thought for {}{}",
                        COLOR_CYAN, duration_text, COLOR_RESET
                    ));

                    self.state.is_planning = false;
                    self.println_with_tracking("");
                }
            }

            AgentEventType::TextMessageStart { role, .. } => {
                // Generate a unique message ID (in a real system this would come from the event)
                let message_id = format!(
                    "msg_{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                );

                // Start a new message
                self.state.add_or_update_message(message_id, role, None);

                // For assistant messages, we'll show content as it streams
                // No immediate output here - wait for content
            }
            AgentEventType::TextMessageContent {
                delta,
                stripped_content,
                ..
            } => {
                self.state.append_to_current_message(&delta);
                // In verbose mode, show positioned content blocks with different colors
                if let Some(stripped_content) = stripped_content {
                    if self.context.verbose {
                        // Print each block with appropriate colors
                        eprint!("{}[PARSER] {}", COLOR_GRAY, COLOR_RESET);
                        for (pos, content) in stripped_content {
                            if content.trim_start().starts_with('<') && content.contains('>') {
                                // Tool call content in red
                                eprint!("{}[{}:{}]{}", COLOR_RED, pos, content, COLOR_RESET);
                            } else {
                                // Regular content in cyan
                                eprint!(
                                    "{}[{}:{}]{}",
                                    COLOR_BRIGHT_CYAN, pos, content, COLOR_RESET
                                );
                            }
                        }
                        eprintln!();
                    }
                }

                // Stream the clean content directly
                self.print_with_tracking(&delta);
            }
            AgentEventType::TextMessageEnd { .. } => {
                self.state.complete_current_message();

                // Ensure we end with a newline like Claude Code
                self.ensure_newline();
                self.println_with_tracking("");
            }
            AgentEventType::ToolExecutionStart {
                step_id: _,
                tool_call_name,
                tool_call_id,
                input,
            } => {
                self.handle_tool_execution_start(&tool_call_name, &tool_call_id, &input);
            }
            AgentEventType::ToolExecutionEnd { tool_call_id, .. } => {
                self.handle_tool_execution_end(&tool_call_id);
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                self.ensure_newline();
                self.println_with_tracking(&format!(
                    "{}→ Agent handover: {} → {}{}",
                    COLOR_CYAN, from_agent, to_agent, COLOR_RESET
                ));
                if let Some(reason) = reason {
                    self.println_with_tracking(&format!("   Reason: {}", reason));
                }
                self.println_with_tracking("");
            }
            AgentEventType::RunFinished {
                success,
                total_steps,
                failed_steps: _,
                usage: _,
            } => {
                self.state.is_streaming = false;
                self.ensure_newline();

                if self.context.verbose {
                    self.print_usage_info().await?;
                    self.println_with_tracking("");

                    if success {
                        self.println_with_tracking(&format!(
                            "{}✓ Agent execution completed ({} steps){}",
                            COLOR_GREEN, total_steps, COLOR_RESET
                        ));
                    } else {
                        self.println_with_tracking(&format!(
                            "{}✗ Agent execution failed{}",
                            COLOR_RED, COLOR_RESET
                        ));
                    }
                }
            }
            AgentEventType::RunError { message, code } => {
                self.state.is_streaming = false;
                self.ensure_newline();

                self.println_with_tracking(&format!(
                    "{}✗ Error: {}{}",
                    COLOR_RED, message, COLOR_RESET
                ));

                if let Some(code) = code {
                    self.println_with_tracking(&format!("   Code: {}", code));
                }

                if self.context.verbose {
                    self.print_usage_info().await?;
                }

                self.println_with_tracking("");
            }
            AgentEventType::StepStarted {
                step_id,
                step_index: idx,
            } => {
                // Create step state but don't print anything yet
                // Steps will be shown when tools execute or messages are generated
                let step_state = StepState {
                    id: step_id.clone(),
                    title: format!("Step {}", idx + 1),
                    index: idx,
                    status: "running".to_string(),
                    start_time: Some(Instant::now()),
                    end_time: None,
                };

                self.state.steps.insert(step_id, step_state);
            }
            AgentEventType::StepCompleted { step_id, success } => {
                // Update step state
                if let Some(step) = self.state.steps.get_mut(&step_id) {
                    step.status = if success {
                        "completed".to_string()
                    } else {
                        "failed".to_string()
                    };
                    step.end_time = Some(Instant::now());
                }

                // No visual output - let the tools and messages speak for themselves
            }
            AgentEventType::TodosUpdated {
                formatted_todos,
                action,
                todo_count,
            } => {
                self.ensure_newline();

                let heading = todo_action_label(action.as_str());
                self.println_with_tracking(&format!(
                    "{}• {}{}",
                    COLOR_BRIGHT_CYAN, heading, COLOR_RESET
                ));

                let todo_lines: Vec<_> = formatted_todos
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty())
                    .collect();

                if todo_count == 0 || todo_lines.is_empty() {
                    self.println_with_tracking("  └ (No todos tracked)");
                } else {
                    for (idx, line) in todo_lines.iter().enumerate() {
                        let prefix = if idx == 0 { "  └" } else { "    " };
                        self.println_with_tracking(&format!("{} {}", prefix, line));
                    }
                }

                self.println_with_tracking("");
            }
            AgentEventType::PlanPruned { removed_steps } => {
                if self.context.verbose {
                    self.ensure_newline();
                    self.println_with_tracking(&format!(
                        "{}→ Plan updated: {} steps removed{}",
                        COLOR_YELLOW,
                        removed_steps.len(),
                        COLOR_RESET
                    ));
                }
            }
            AgentEventType::BrowserScreenshot {
                image,
                format,
                filename,
                size,
                timestamp_ms,
            } => {
                self.show_browser_screenshot(
                    &image,
                    format.as_deref(),
                    filename.as_deref(),
                    size,
                    timestamp_ms,
                );
            }
            AgentEventType::ToolCalls { .. } => {}
            AgentEventType::ToolResults { results, .. } => {
                // Update tool call states with results
                for result in results {
                    // Check if the result indicates success (no explicit success field, assume success if no error in result)
                    let status = ToolCallStatus::Completed; // For now, assume success

                    self.state.update_tool_call_status(
                        &result.tool_call_id,
                        status,
                        Some(result.result().clone()),
                        None, // No error field in ToolResponse
                    );
                }
            }
            AgentEventType::WorkflowStarted {
                workflow_name,
                total_steps,
            } => {
                self.ensure_newline();
                self.println_with_tracking(&format!(
                    "{}→ Starting workflow: {} ({} steps){}",
                    COLOR_CYAN, workflow_name, total_steps, COLOR_RESET
                ));
                self.println_with_tracking("");
            }
            AgentEventType::NodeStarted {
                node_id: _,
                node_name,
                step_type,
            } => {
                if self.context.verbose {
                    self.println_with_tracking(&format!(
                        "{}→ Starting step: {} ({}){}",
                        COLOR_GRAY, node_name, step_type, COLOR_RESET
                    ));
                }
            }
            AgentEventType::NodeCompleted {
                node_id: _,
                node_name,
                success,
                error,
            } => {
                if self.context.verbose {
                    if success {
                        self.println_with_tracking(&format!(
                            "{}✓ Completed step: {}{}",
                            COLOR_GREEN, node_name, COLOR_RESET
                        ));
                    } else {
                        self.println_with_tracking(&format!(
                            "{}✗ Failed step: {} - {}{}",
                            COLOR_RED,
                            node_name,
                            error.as_deref().unwrap_or("unknown error"),
                            COLOR_RESET
                        ));
                    }
                }
            }
            AgentEventType::RunCompleted {
                workflow_name,
                success,
                total_steps,
            } => {
                self.ensure_newline();
                if success {
                    self.println_with_tracking(&format!(
                        "{}✓ Workflow completed: {} ({} steps){}",
                        COLOR_GREEN, workflow_name, total_steps, COLOR_RESET
                    ));
                } else {
                    self.println_with_tracking(&format!(
                        "{}✗ Workflow failed: {} ({} steps){}",
                        COLOR_RED, workflow_name, total_steps, COLOR_RESET
                    ));
                }
                self.println_with_tracking("");
            }
            AgentEventType::RunFailed {
                workflow_name,
                error,
                failed_at_step,
            } => {
                self.ensure_newline();
                if let Some(step) = failed_at_step {
                    self.println_with_tracking(&format!(
                        "{}✗ Workflow failed: {} at step '{}' - {}{}",
                        COLOR_RED, workflow_name, step, error, COLOR_RESET
                    ));
                } else {
                    self.println_with_tracking(&format!(
                        "{}✗ Workflow failed: {} - {}{}",
                        COLOR_RED, workflow_name, error, COLOR_RESET
                    ));
                }
                self.println_with_tracking("");
            }
            AgentEventType::InlineHookRequested { .. } => {
                // Inline hooks are handled asynchronously; no direct CLI output needed.
            }
            AgentEventType::BrowserSessionStarted { .. } => {
                // Browser Session Started
            }
        };

        Ok(())
    }

    pub fn finish(&mut self) {
        // Ensure clean completion
        self.ensure_newline();

        // Complete any ongoing streaming
        self.state.complete_current_message();

        // Clear any remaining tool call states
        self.state.tool_calls.clear();

        // Final newline for clean terminal
        if !self.state.tool_calls.is_empty() || self.state.is_streaming {
            self.println_with_tracking("");
        }
    }

    fn show_browser_screenshot(
        &mut self,
        image_data: &str,
        format: Option<&str>,
        filename: Option<&str>,
        size: Option<u64>,
        timestamp_ms: Option<i64>,
    ) {
        self.ensure_newline();
        self.println_with_tracking(&format!(
            "{}🖥  Browser preview{}",
            COLOR_DISTRI_TEAL, COLOR_RESET
        ));

        if let Some(metadata_line) =
            Self::format_browser_metadata(format, filename, size, timestamp_ms)
        {
            self.println_with_tracking(&metadata_line);
        }

        if let Err(err) = self.print_browser_image(image_data) {
            self.println_with_tracking(&format!(
                "{}   Unable to render screenshot: {}{}",
                COLOR_YELLOW, err, COLOR_RESET
            ));
        }

        self.println_with_tracking("");
    }

    fn print_browser_image(&mut self, encoded_image: &str) -> Result<(), anyhow::Error> {
        let snapshot = Self::decode_browser_image(encoded_image)?;
        let (cols, _) = terminal::size().unwrap_or((100, 40));
        let mut width_cells = cols.saturating_sub(4);
        if width_cells == 0 {
            width_cells = cols;
        }
        if width_cells == 0 {
            width_cells = 80;
        }
        width_cells = width_cells.min(160);

        // Scale browser previews down so they don't dominate CLI output
        let preview_width = width_cells.min(80);
        let config = Config {
            width: Some(u32::from(preview_width)),
            ..Default::default()
        };

        viuer::print(&snapshot, &config)
            .map_err(|err| anyhow!("failed to display browser screenshot: {}", err))?;

        println!();
        self.last_output_was_newline = true;
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

        let snapshot = image::load_from_memory(&decoded)
            .context("failed to parse browser screenshot bytes")?;

        Ok(snapshot)
    }

    fn format_browser_metadata(
        format: Option<&str>,
        filename: Option<&str>,
        size: Option<u64>,
        timestamp_ms: Option<i64>,
    ) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(name) = filename {
            parts.push(format!("File: {}", name));
        }

        if let Some(fmt) = format {
            if !fmt.is_empty() {
                parts.push(fmt.to_uppercase());
            }
        }

        if let Some(bytes) = size {
            parts.push(format!("{}", Self::format_size(bytes)));
        }

        if let Some(timestamp) = Self::format_browser_timestamp(timestamp_ms) {
            parts.push(format!("Captured {}", timestamp));
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!(
                "   {}{}{}",
                COLOR_GRAY,
                parts.join(" • "),
                COLOR_RESET
            ))
        }
    }

    fn format_size(size: u64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;

        if size < 1024 {
            format!("{} B", size)
        } else if (size as f64) < MB {
            format!("{:.1} KB", (size as f64) / KB)
        } else {
            format!("{:.1} MB", (size as f64) / MB)
        }
    }

    fn format_browser_timestamp(timestamp_ms: Option<i64>) -> Option<String> {
        let ms = timestamp_ms?;
        let seconds = ms.div_euclid(1000);
        let millis = ms.rem_euclid(1000) as u32;
        let nanos = millis * 1_000_000;

        DateTime::<Utc>::from_timestamp(seconds, nanos).map(|ts| {
            ts.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
    }
}

pub async fn run_stream_with_printer(
    agent_name: &str,
    executor: Arc<AgentOrchestrator>,
    task: Message,
    verbose: bool,
    thread_id: Option<String>,
    current_model: Option<&str>,
    user_id: Option<&str>,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
) -> anyhow::Result<Option<InvokeResult>> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<distri_core::agent::AgentEvent>(100);

    let thread_id = thread_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let task_id = uuid::Uuid::new_v4().to_string(); // Each CLI execution is a separate task
    let context = ExecutorContext {
        thread_id: thread_id.clone(),
        task_id,
        run_id: uuid::Uuid::new_v4().to_string(),
        verbose,
        session_id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.unwrap_or("cli_user").to_string(),
        orchestrator: Some(executor.clone()),
        stores: Some(executor.stores.clone()),
        agent_id: agent_name.to_string(),
        event_tx: Some(Arc::new(tx)),
        ..Default::default()
    };

    let context = Arc::new(context);

    // Get agent definition and max_iterations
    let agent_definition = executor.get_agent(agent_name).await;
    let max_iterations = match &agent_definition {
        Some(distri_types::configuration::AgentConfig::StandardAgent(def)) => {
            def.max_iterations.unwrap_or(10)
        }

        _ => 10, // Default fallback
    };

    // Create definition overrides if current_model is provided
    let definition_overrides =
        current_model.map(|model| DefinitionOverrides::new().with_model(model.to_string()));

    let agent_name = agent_name.to_string();
    let agent_name_clone = agent_name.clone();
    let context_clone = context.clone();
    let handle = tokio::spawn(async move {
        let res = executor
            .execute_stream(&agent_name_clone, task, context_clone, definition_overrides)
            .await;

        match res {
            Ok(res) => res.content.clone().unwrap_or_default(),
            Err(e) => e.to_string(),
        }
    });

    let mut printer =
        EventPrinter::new_with_context(context.clone(), max_iterations, tool_renderers.clone());

    // Set agent definition in printer if available
    if let Some(distri_types::configuration::AgentConfig::StandardAgent(def)) = agent_definition {
        printer.set_agent_definition(def);
    }

    // Move receiver to background and get a handle to control it
    let handle2 = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Err(e) = printer.handle_event(event.clone()).await {
                eprintln!("Error handling event: {}", e);
            }
        }
        debug!("Receiver task exiting");
        printer.finish();
        if printer.context.verbose {
            let _ = printer.print_context_size_summary().await;
        }
    });

    // Wait for main agent execution to complete
    match handle.await {
        Ok(result) => {
            println!("{result}");
        }
        Err(join_error) => {
            handle2.abort();
            return Err(join_error).context("agent execution task failed");
        }
    }

    // Explicitly abort the receiver task since we're done
    handle2.abort();

    // Wait for the abort to complete
    if let Err(e) = handle2.await {
        debug!("Receiver task was aborted : {:?}", e);
    }

    // Return empty result since we're now focused on clean output rather than tool call tracking
    Ok(Some(InvokeResult {
        content: None,
        tool_calls: Vec::new(),
    }))
}
