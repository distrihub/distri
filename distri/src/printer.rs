use std::{collections::HashMap, sync::Arc, time::Instant};

use chrono::Local;
use distri_a2a::MessageSendParams;
use distri_types::{AgentEvent, AgentEventType, MessageRole, ToolResponse};
use tokio::sync::{Mutex, RwLock};

use crate::client_stream::{AgentStreamClient, StreamError, StreamItem};

// Re-export shared color constants from the formatter
pub use distri_formatter::colors::{
    COLOR_BRIGHT_CYAN, COLOR_CYAN, COLOR_GRAY, COLOR_RED, COLOR_RESET, COLOR_YELLOW,
};

/// Shared context health state — updated by the event printer, read by the CLI status line.
#[derive(Debug, Clone, Default)]
pub struct ContextHealth {
    /// Context utilization as a percentage (0.0–1.0)
    pub utilization: f64,
    /// Total tokens used
    pub tokens_used: usize,
    /// Total context window size
    pub tokens_limit: usize,
    /// Whether in warning state (>80%)
    pub is_warning: bool,
    /// Whether in critical state (>90%)
    pub is_critical: bool,
    /// Last model used
    pub model: Option<String>,
    /// Cumulative input tokens from API
    pub api_input_tokens: u32,
    /// Cumulative output tokens from API
    pub api_output_tokens: u32,
    /// Cached tokens
    pub api_cached_tokens: u32,
    /// Estimated cost in USD
    pub cost_usd: Option<f64>,
    /// Last received full context budget breakdown
    pub last_budget: Option<distri_types::ContextBudget>,
}

impl ContextHealth {
    /// Update from a ContextBudget
    pub fn update_from_budget(&mut self, budget: &distri_types::ContextBudget) {
        self.utilization = budget.utilization();
        self.tokens_used = budget.total_tokens();
        self.tokens_limit = budget.context_window_size;
        self.is_warning = budget.is_warning();
        self.is_critical = budget.is_critical();
        self.last_budget = Some(budget.clone());
    }

    /// Print a detailed context breakdown to stdout.
    pub fn print_context_breakdown(&self) {
        let Some(ref budget) = self.last_budget else {
            println!("No context data yet — run a query first.");
            return;
        };
        if budget.context_window_size == 0 {
            println!("Context window size unknown.");
            return;
        }
        let total = budget.total_tokens();
        let window = budget.context_window_size;
        let pct = budget.utilization() * 100.0;
        let remaining = budget.remaining_tokens();

        println!(
            "{}{:.0}% used — {} / {} tokens  ({} remaining){}",
            COLOR_GRAY,
            pct,
            format_token_count(total),
            format_token_count(window),
            format_token_count(remaining),
            COLOR_RESET
        );

        let rows: &[(&str, usize)] = &[
            ("system (static)", budget.system_prompt_static_tokens),
            ("system (dynamic)", budget.system_prompt_dynamic_tokens),
            ("tool schemas", budget.tool_schema_tokens),
            ("deferred tools", budget.deferred_tool_tokens),
            ("skills", budget.skill_listing_tokens),
            ("conversation", budget.conversation_tokens),
            ("tool results", budget.tool_result_tokens),
        ];

        let max_label = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
        for (label, tokens) in rows {
            if *tokens == 0 {
                continue;
            }
            let bar_pct = if window > 0 { *tokens * 40 / window } else { 0 };
            let bar = "█".repeat(bar_pct);
            println!(
                "{}  {:<width$}  {:>6}  {}{}",
                COLOR_GRAY,
                label,
                format_token_count(*tokens),
                bar,
                COLOR_RESET,
                width = max_label
            );
        }
    }

    /// Reset per-run token counters. Called on RunStarted so the status bar
    /// shows "this run" totals instead of accumulating across the whole session.
    pub fn reset_run_tokens(&mut self) {
        self.api_input_tokens = 0;
        self.api_output_tokens = 0;
        self.api_cached_tokens = 0;
        self.cost_usd = None;
    }

    /// Update from RunUsage (per-step delta — adds to current run total).
    pub fn update_from_usage(&mut self, usage: &distri_types::events::RunUsage) {
        self.api_input_tokens = self.api_input_tokens.saturating_add(usage.input_tokens);
        self.api_output_tokens = self.api_output_tokens.saturating_add(usage.output_tokens);
        self.api_cached_tokens = self.api_cached_tokens.saturating_add(usage.cached_tokens);
        if let Some(model) = &usage.model {
            self.model = Some(model.clone());
        }
        if let Some(cost) = usage.cost_usd {
            *self.cost_usd.get_or_insert(0.0) += cost;
        }
    }

    /// Format as a compact status line string for the terminal.
    /// Example: "Context: 45% (9.2K/20K) · Tokens: 1.2K in, 340 out · $0.03"
    pub fn format_status_line(&self) -> String {
        if self.tokens_limit == 0 && self.api_input_tokens == 0 {
            return String::new(); // No data yet
        }

        let mut parts = Vec::new();

        // Context utilization
        if self.tokens_limit > 0 {
            let pct = (self.utilization * 100.0) as u32;
            let used = format_token_count(self.tokens_used);
            let limit = format_token_count(self.tokens_limit);
            parts.push(format!("Context: {}% ({}/{})", pct, used, limit));
        }

        // API token usage
        if self.api_input_tokens > 0 || self.api_output_tokens > 0 {
            let input = format_token_count(self.api_input_tokens as usize);
            let output = format_token_count(self.api_output_tokens as usize);
            let mut tok = format!("Tokens: {} in, {} out", input, output);
            if self.api_cached_tokens > 0 {
                tok.push_str(&format!(
                    " ({} cached)",
                    format_token_count(self.api_cached_tokens as usize)
                ));
            }
            parts.push(tok);
        }

        // Cost
        if let Some(cost) = self.cost_usd {
            parts.push(format!("${:.2}", cost));
        }

        parts.join(" · ")
    }

    /// ANSI color for the current utilization level.
    pub fn color(&self) -> &'static str {
        if self.is_critical {
            "\x1b[38;2;239;68;68m" // Red
        } else if self.is_warning {
            "\x1b[38;2;249;115;22m" // Orange
        } else if self.utilization > 0.5 {
            "\x1b[38;2;234;179;8m" // Yellow
        } else {
            "\x1b[38;2;34;197;94m" // Green
        }
    }
}

/// Format a token count as "1.2K", "45K", "200K", or "3" for small values.
fn format_token_count(count: usize) -> String {
    if count >= 1000 {
        let k = count as f64 / 1000.0;
        if k >= 100.0 {
            format!("{}K", k as usize)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        format!("{}", count)
    }
}
/// Print a compact `↳ Xk in, Xk out [· Xk cached] [· $0.0031] [· model]` usage line.
fn print_usage_line(u: &distri_types::RunUsage) {
    let input = format_token_count(u.input_tokens as usize);
    let output = format_token_count(u.output_tokens as usize);
    let mut parts = vec![format!("{} in, {} out", input, output)];
    if u.cached_tokens > 0 {
        parts.push(format!(
            "{} cached",
            format_token_count(u.cached_tokens as usize)
        ));
    }
    if let Some(cost) = u.cost_usd {
        parts.push(format!("${:.4}", cost));
    }
    if let Some(model) = &u.model {
        parts.push(model.clone());
    }
    println!("{}  ↳ {}{}", COLOR_GRAY, parts.join(" · "), COLOR_RESET);
}

/// Distri brand teal — 24-bit ANSI
pub const COLOR_DISTRI: &str = "\x1b[38;2;0;124;145m";
pub const COLOR_DISTRI_DIM: &str = "\x1b[38;2;0;80;95m";
pub const COLOR_DISTRI_BRIGHT: &str = "\x1b[38;2;0;180;210m";

/// Animated spinner frames using distri brand colors.
/// Each frame is a set of 4 dots where the "bright" one moves across.
fn distri_spinner_frame(frame: usize) -> String {
    let dots = ['●', '●', '●', '●'];
    let pos = frame % 4;
    let mut result = String::new();
    for (i, dot) in dots.iter().enumerate() {
        if i == pos {
            result.push_str(COLOR_DISTRI_BRIGHT);
        } else if i == (pos + 3) % 4 {
            result.push_str(COLOR_DISTRI_DIM);
        } else {
            result.push_str(COLOR_DISTRI);
        }
        result.push(*dot);
    }
    result.push_str(COLOR_RESET);
    result
}

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
    /// Text shown on the transient planning line (cleared before other output).
    planning_text: Option<String>,
    /// Frame counter for spinner animation (advances each redraw).
    spinner_frame: usize,
    /// Shared context health state — updated by events, read by CLI status line.
    pub context_health: Arc<RwLock<ContextHealth>>,
}

impl Default for EventPrinter {
    fn default() -> Self {
        Self::new()
    }
}

impl EventPrinter {
    pub fn new() -> Self {
        Self {
            state: ChatState::default(),
            verbose: false,
            show_tools: true,
            agent_display_name: None,
            context_health: Arc::new(RwLock::new(ContextHealth::default())),
            planning_text: None,
            spinner_frame: 0,
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

    /// Show a planning line with animated spinner. Redraws on same line.
    fn show_planning(&mut self, text: String) {
        self.clear_planning_line();
        self.planning_text = Some(text);
        self.state.is_planning = true;
        self.redraw_planning();
    }

    /// Redraw the planning line with the next animation frame.
    fn redraw_planning(&mut self) {
        if let Some(ref text) = self.planning_text {
            let spinner = distri_spinner_frame(self.spinner_frame);
            print!(
                "\r\x1b[2K {} {}{}{}",
                spinner, COLOR_GRAY, text, COLOR_RESET
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
            self.spinner_frame += 1;
        }
    }

    /// Clear the planning/spinner line completely.
    fn clear_planning_line(&mut self) {
        if self.state.is_planning || self.planning_text.is_some() {
            print!("\r\x1b[2K");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            self.planning_text = None;
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

        // Clear the spinner before any other event prints output
        match &event.event {
            AgentEventType::PlanStarted { .. } | AgentEventType::PlanFinished { .. } => {}
            _ => self.clear_planning_line(),
        }

        match &event.event {
            AgentEventType::RunStarted {} => {
                // Reset per-run token counters so the status bar shows this-run totals.
                self.context_health.write().await.reset_run_tokens();
            }
            AgentEventType::PlanStarted { initial_plan } => {
                self.state.is_planning = true;
                let idx = self.state.steps.len();
                let phrase = if *initial_plan {
                    distri_types::thinking::pick_planning(idx)
                } else {
                    distri_types::thinking::pick_replanning(idx)
                };
                self.show_planning(format!("{} {}…", phrase.emoji, phrase.text));
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
            AgentEventType::StepCompleted {
                step_id,
                success,
                context_budget,
                usage,
            } => {
                if let Some(budget) = context_budget {
                    let mut health = self.context_health.write().await;
                    health.update_from_budget(budget);
                }
                if let Some(u) = usage {
                    self.context_health.write().await.update_from_usage(u);
                    if u.input_tokens > 0 || u.output_tokens > 0 {
                        print_usage_line(u);
                    }
                }
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
            AgentEventType::RunFinished {
                success,
                usage,
                context_budget,
                ..
            } => {
                let mut health = self.context_health.write().await;
                if let Some(budget) = context_budget {
                    health.update_from_budget(budget);
                }
                if let Some(u) = usage {
                    health.update_from_usage(u);
                }
                drop(health);
                if !success {
                    println!("{}Run completed with errors{}", COLOR_RED, COLOR_RESET);
                }
            }

            AgentEventType::ContextBudgetUpdate {
                budget,
                is_warning,
                is_critical,
            } => {
                let mut health = self.context_health.write().await;
                health.update_from_budget(budget);
                drop(health);
                // Print a warning if crossing thresholds
                if *is_critical {
                    println!(
                        "{}⚠ Context critical: {:.0}% used ({}/{}){}",
                        COLOR_RED,
                        budget.utilization() * 100.0,
                        format_token_count(budget.total_tokens()),
                        format_token_count(budget.context_window_size),
                        COLOR_RESET
                    );
                } else if *is_warning {
                    println!(
                        "{}⚠ Context warning: {:.0}% used{}",
                        COLOR_YELLOW,
                        budget.utilization() * 100.0,
                        COLOR_RESET
                    );
                }
                // Verbose: full breakdown of context budget components
                if self.verbose && budget.context_window_size > 0 {
                    println!(
                        "{}[context] {:.0}% ({}/{})",
                        COLOR_GRAY,
                        budget.utilization() * 100.0,
                        format_token_count(budget.total_tokens()),
                        format_token_count(budget.context_window_size),
                    );
                    println!(
                        "{}  sys static: {}  dynamic: {}  tools: {}  deferred: {}  skills: {}{}",
                        COLOR_GRAY,
                        format_token_count(budget.system_prompt_static_tokens),
                        format_token_count(budget.system_prompt_dynamic_tokens),
                        format_token_count(budget.tool_schema_tokens),
                        format_token_count(budget.deferred_tool_tokens),
                        format_token_count(budget.skill_listing_tokens),
                        COLOR_RESET,
                    );
                    println!(
                        "{}  convo: {}  tool_results: {}{}",
                        COLOR_GRAY,
                        format_token_count(budget.conversation_tokens),
                        format_token_count(budget.tool_result_tokens),
                        COLOR_RESET,
                    );
                }
            }
            AgentEventType::RunError {
                message,
                code,
                usage,
                ..
            } => {
                let stamp = Local::now().format("%H:%M:%S").to_string();
                println!(
                    "{}{} [{}] run failed: {} ({:?}){}",
                    COLOR_RED, stamp, event.agent_id, message, code, COLOR_RESET
                );
                if let Some(u) = usage {
                    print_usage_line(u);
                }
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
            AgentEventType::LiveView { url, title, .. } => {
                let label = title.as_deref().unwrap_or("Live view");
                println!("{}⎔ {}: {}{}", COLOR_BRIGHT_CYAN, label, url, COLOR_RESET);
            }
            AgentEventType::ContextCompaction {
                tier,
                tokens_before,
                tokens_after,
                entries_affected,
                ..
            } => {
                let tier_label = match tier {
                    distri_types::CompactionTier::Trim => "Trim",
                    distri_types::CompactionTier::Summarize => "Summarize",
                    distri_types::CompactionTier::Reset => "Reset",
                };
                println!(
                    "{}[compaction] {}: {} → {} tokens ({:.0}% reduction, {} entries){}",
                    COLOR_GRAY,
                    tier_label,
                    format_token_count(*tokens_before),
                    format_token_count(*tokens_after),
                    if *tokens_before > 0 {
                        (1.0 - *tokens_after as f64 / *tokens_before as f64) * 100.0
                    } else {
                        0.0
                    },
                    entries_affected,
                    COLOR_RESET
                );
            }
            AgentEventType::DiagnosticLog { message } => {
                if self.verbose {
                    println!("{}[dbg] {}{}", COLOR_GRAY, message, COLOR_RESET);
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

    fn tool_start(&mut self, tool_call_id: &str, name: &str, input: &serde_json::Value) {
        if self.show_tools && !distri_formatter::state::is_probe_call(name, input) {
            println!(
                "{}⏺ {}{}",
                COLOR_YELLOW,
                distri_formatter::state::format_tool_call(name, input),
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

    fn handle_tool_calls(&mut self, _tool_calls: &[distri_types::ToolCall], _parent: Option<&str>) {
        // Suppressed — individual ToolExecutionStart events show each call
    }

    fn print_tool_result(&self, result: &ToolResponse) {
        if !self.show_tools {
            return;
        }
        crate::renderers::render_tool_output(result, self.verbose);
    }

    fn format_tool_input(&self, input: &serde_json::Value) -> String {
        distri_formatter::state::format_tool_input(input)
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
    let (_, result) = print_stream_with_health(
        client,
        agent_id,
        params,
        verbose,
        agent_display_name,
        show_tools,
        None,
    )
    .await?;
    result
}

/// Stream agent events, print to terminal, and update shared context health state.
///
/// If `shared_health` is Some, updates it from budget events so the caller
/// (e.g., the chat loop) can display context utilization in the status line.
///
/// Returns (updated_health, stream_result).
pub async fn print_stream_with_health(
    client: &AgentStreamClient,
    agent_id: &str,
    params: MessageSendParams,
    verbose: bool,
    agent_display_name: Option<String>,
    show_tools: bool,
    shared_health: Option<Arc<RwLock<ContextHealth>>>,
) -> Result<(Arc<RwLock<ContextHealth>>, Result<(), StreamError>), StreamError> {
    let mut printer = EventPrinter::new().with_verbose(verbose);
    if let Some(name) = agent_display_name {
        printer = printer.with_agent_name(name);
    }
    if !show_tools {
        printer.toggle_show_tools();
    }
    // Share the health state between the printer and the caller
    if let Some(ref health) = shared_health {
        printer.context_health = health.clone();
    }
    let health_out = printer.context_health.clone();
    let printer = Arc::new(Mutex::new(printer));
    let result = client
        .stream_agent(agent_id, params, {
            let printer = printer.clone();
            move |item: StreamItem| {
                let printer = printer.clone();
                async move {
                    if let Some(event) = item.agent_event.clone() {
                        let mut guard = printer.lock().await;
                        guard.handle_event(&event).await;
                    }
                    if let Some(ref msg) = item.message
                        && msg.role == distri_types::MessageRole::Assistant
                        && let Some(text) = msg.as_text()
                        && !text.is_empty()
                    {
                        println!("\n{}", text);
                    }
                }
            }
        })
        .await;
    Ok((health_out, result))
}
