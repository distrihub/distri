use anyhow::Result;
use chrono::Utc;
use crossterm::terminal;
use distri::{Distri, TraceSummary};
use distri_types::{Message, Part};

use crate::{
    OptimizeCommands, TracesCommands, COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_GRAY,
    COLOR_RESET,
};

// ─────────────────────────────────────────────────────────────────────────────
// ANSI color constants for span categories
// ─────────────────────────────────────────────────────────────────────────────

const COLOR_PURPLE: &str = "\x1b[35m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_BLUE: &str = "\x1b[34m";
const COLOR_CYAN: &str = "\x1b[36m";
const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_BRIGHT_YELLOW: &str = "\x1b[93m";
const COLOR_BRIGHT_BLUE: &str = "\x1b[94m";
const COLOR_BRIGHT_CYAN: &str = "\x1b[96m";
const COLOR_WHITE: &str = "\x1b[37m";
const COLOR_DIM: &str = "\x1b[2m";
const COLOR_BOLD: &str = "\x1b[1m";

// ─────────────────────────────────────────────────────────────────────────────
// Span category
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum SpanCategory {
    LlmCall,
    ToolExecution,
    AgentInvocation,
    ChainOperation,
    Plan,
    Step,
    AgentHandover,
    Guardrail,
    Unknown,
}

impl SpanCategory {
    fn label(&self) -> &str {
        match self {
            Self::LlmCall => "LLM",
            Self::ToolExecution => "Tool",
            Self::AgentInvocation => "Agent",
            Self::ChainOperation => "Chain",
            Self::Plan => "Plan",
            Self::Step => "Step",
            Self::AgentHandover => "Handover",
            Self::Guardrail => "Guard",
            Self::Unknown => "Span",
        }
    }

    fn color(&self) -> &str {
        match self {
            Self::LlmCall => COLOR_PURPLE,
            Self::ToolExecution => COLOR_YELLOW,
            Self::AgentInvocation => COLOR_BLUE,
            Self::ChainOperation => COLOR_GREEN,
            Self::Plan => COLOR_CYAN,
            Self::Step => COLOR_WHITE,
            Self::AgentHandover => COLOR_BRIGHT_MAGENTA,
            Self::Guardrail => COLOR_BRIGHT_YELLOW,
            Self::Unknown => COLOR_GRAY,
        }
    }

    fn bar_char(&self) -> &str {
        match self {
            Self::LlmCall => "█",
            Self::ToolExecution => "█",
            Self::AgentInvocation => "█",
            Self::ChainOperation => "█",
            Self::Plan => "█",
            Self::Step => "█",
            Self::AgentHandover => "█",
            Self::Guardrail => "█",
            Self::Unknown => "█",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsed span
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CliSpan {
    span_id: String,
    parent_span_id: Option<String>,
    name: String,
    start_time_ns: i64,
    end_time_ns: i64,
    attributes: Vec<(String, String)>,
    children: Vec<CliSpan>,
}

impl CliSpan {
    fn duration_ns(&self) -> i64 {
        self.end_time_ns - self.start_time_ns
    }

    fn get_attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn category(&self) -> SpanCategory {
        // Check gen_ai.operation.name first for custom distri operations
        if let Some(op) = self.get_attr("gen_ai.operation.name") {
            match op {
                "plan" => return SpanCategory::Plan,
                "step" => return SpanCategory::Step,
                "agent_handover" => return SpanCategory::AgentHandover,
                "execute" => return SpanCategory::AgentInvocation,
                _ => {}
            }
        }

        // Detect from openinference.span.kind or name patterns
        if let Some(kind) = self.get_attr("openinference.span.kind") {
            match kind.to_uppercase().as_str() {
                "LLM" => return SpanCategory::LlmCall,
                "TOOL" => return SpanCategory::ToolExecution,
                "AGENT" => return SpanCategory::AgentInvocation,
                "CHAIN" => return SpanCategory::ChainOperation,
                "RETRIEVER" => return SpanCategory::ChainOperation,
                "EMBEDDING" => return SpanCategory::LlmCall,
                "GUARDRAIL" => return SpanCategory::Guardrail,
                _ => {}
            }
        }

        // Detect from gen_ai.operation.name for standard operations
        if let Some(op) = self.get_attr("gen_ai.operation.name") {
            match op {
                "chat" | "completion" => return SpanCategory::LlmCall,
                _ => {}
            }
        }

        // Detect LLM by model attribute
        if self.get_attr("gen_ai.request.model").is_some() {
            return SpanCategory::LlmCall;
        }

        // Detect tool by name
        if self.get_attr("gen_ai.tool.call.arguments").is_some() {
            return SpanCategory::ToolExecution;
        }

        SpanCategory::Unknown
    }

    fn clean_title(&self) -> String {
        let cat = self.category();
        let op = self.get_attr("gen_ai.operation.name");

        match cat {
            SpanCategory::LlmCall => {
                // Use model name if available
                if let Some(model) = self.get_attr("gen_ai.request.model") {
                    return model.to_string();
                }
                // Title is "model - span_name", take just the model
                if let Some(idx) = self.name.find(" - ") {
                    return self.name[..idx].to_string();
                }
                self.name.clone()
            }
            SpanCategory::AgentInvocation => self
                .name
                .replace("invoke_agent ", "")
                .replace("execute ", ""),
            SpanCategory::ToolExecution => self.name.replace("execute_tool ", ""),
            SpanCategory::Plan => {
                if self.name.contains("initial") {
                    "Planning (initial)".to_string()
                } else {
                    "Re-planning".to_string()
                }
            }
            SpanCategory::Step => {
                if let Some(op_name) = op {
                    if op_name == "step" {
                        // "step 0" → "Step 0"
                        let title = self.name.replace("step ", "Step ");
                        if title == self.name {
                            format!("Step {}", self.name)
                        } else {
                            title
                        }
                    } else {
                        self.name.clone()
                    }
                } else {
                    self.name.clone()
                }
            }
            SpanCategory::AgentHandover => self.name.replace("handover ", ""),
            _ => self.name.clone(),
        }
    }

    fn input_tokens(&self) -> Option<i64> {
        self.get_attr("gen_ai.usage.input_tokens")
            .and_then(|v| v.parse().ok())
    }

    fn cost(&self) -> Option<f64> {
        self.get_attr("gen_ai.usage.cost")
            .and_then(|v| v.parse().ok())
    }

    fn input_value(&self) -> Option<&str> {
        self.get_attr("input.value")
    }

    fn output_value(&self) -> Option<&str> {
        self.get_attr("output.value")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OTLP JSON → CliSpan tree
// ─────────────────────────────────────────────────────────────────────────────

fn parse_otlp_spans(otlp: &serde_json::Value) -> Vec<CliSpan> {
    let mut flat: Vec<CliSpan> = Vec::new();

    let resource_spans = match otlp.get("resourceSpans").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return flat,
    };

    for rs in resource_spans {
        let scope_spans = match rs.get("scopeSpans").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for ss in scope_spans {
            let spans = match ss.get("spans").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for span_obj in spans {
                if let Some(cli_span) = parse_single_span(span_obj) {
                    flat.push(cli_span);
                }
            }
        }
    }

    build_tree(flat)
}

fn parse_single_span(v: &serde_json::Value) -> Option<CliSpan> {
    let span_id = v.get("spanId")?.as_str()?.to_string();
    let parent_raw = v.get("parentSpanId").and_then(|v| v.as_str()).unwrap_or("");
    let parent_span_id = if parent_raw.is_empty() {
        None
    } else {
        Some(parent_raw.to_string())
    };
    let name = v.get("name")?.as_str()?.to_string();

    let start_time_ns = v
        .get("startTimeUnixNano")
        .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
        .and_then(|s| {
            if s.is_empty() {
                v.get("startTimeUnixNano").and_then(|v| v.as_i64())
            } else {
                s.parse().ok()
            }
        })
        .unwrap_or(0);

    let end_time_ns = v
        .get("endTimeUnixNano")
        .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
        .and_then(|s| {
            if s.is_empty() {
                v.get("endTimeUnixNano").and_then(|v| v.as_i64())
            } else {
                s.parse().ok()
            }
        })
        .unwrap_or(0);

    let mut attributes = Vec::new();
    if let Some(attrs) = v.get("attributes").and_then(|v| v.as_array()) {
        for attr in attrs {
            if let (Some(key), Some(value)) =
                (attr.get("key").and_then(|v| v.as_str()), attr.get("value"))
            {
                let val_str = extract_otlp_value(value);
                attributes.push((key.to_string(), val_str));
            }
        }
    }

    Some(CliSpan {
        span_id,
        parent_span_id,
        name,
        start_time_ns,
        end_time_ns,
        attributes,
        children: Vec::new(),
    })
}

fn extract_otlp_value(v: &serde_json::Value) -> String {
    if let Some(s) = v.get("stringValue").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(s) = v.get("intValue").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(n) = v.get("intValue").and_then(|v| v.as_i64()) {
        return n.to_string();
    }
    if let Some(n) = v.get("doubleValue").and_then(|v| v.as_f64()) {
        return n.to_string();
    }
    if let Some(b) = v.get("boolValue").and_then(|v| v.as_bool()) {
        return b.to_string();
    }
    v.to_string()
}

fn build_tree(mut flat: Vec<CliSpan>) -> Vec<CliSpan> {
    use std::collections::HashMap;

    // Sort by start time
    flat.sort_by_key(|s| s.start_time_ns);

    // Index by span_id
    let mut map: HashMap<String, CliSpan> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for span in flat {
        order.push(span.span_id.clone());
        map.insert(span.span_id.clone(), span);
    }

    // Collect parent-child relationships
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut root_ids: Vec<String> = Vec::new();

    for id in &order {
        let span = map.get(id).unwrap();
        if let Some(ref parent_id) = span.parent_span_id {
            if map.contains_key(parent_id) {
                children_map
                    .entry(parent_id.clone())
                    .or_default()
                    .push(id.clone());
            } else {
                root_ids.push(id.clone());
            }
        } else {
            root_ids.push(id.clone());
        }
    }

    // Build tree recursively
    fn assemble(
        id: &str,
        map: &mut HashMap<String, CliSpan>,
        children_map: &HashMap<String, Vec<String>>,
    ) -> Option<CliSpan> {
        let mut span = map.remove(id)?;
        if let Some(child_ids) = children_map.get(id) {
            for cid in child_ids {
                if let Some(child) = assemble(cid, map, children_map) {
                    span.children.push(child);
                }
            }
            span.children.sort_by_key(|c| c.start_time_ns);
        }
        Some(span)
    }

    let mut roots = Vec::new();
    for id in &root_ids {
        if let Some(root) = assemble(id, &mut map, &children_map) {
            roots.push(root);
        }
    }
    roots.sort_by_key(|r| r.start_time_ns);
    roots
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatting helpers
// ─────────────────────────────────────────────────────────────────────────────

fn format_duration_ns(ns: i64) -> String {
    if ns <= 0 {
        return "0ms".to_string();
    }
    let ms = ns as f64 / 1_000_000.0;
    if ms < 1.0 {
        return "<1ms".to_string();
    }
    if ms < 1000.0 {
        return format!("{}ms", ms.round() as i64);
    }
    let secs = ms / 1000.0;
    if secs < 60.0 {
        return format!("{:.1}s", secs);
    }
    let mins = (secs / 60.0).floor() as i64;
    let remaining_secs = (secs - (mins as f64 * 60.0)).round() as i64;
    if mins < 60 {
        if remaining_secs > 0 {
            return format!("{}m {}s", mins, remaining_secs);
        }
        return format!("{}m", mins);
    }
    let hours = mins / 60;
    let remaining_mins = mins % 60;
    if remaining_mins > 0 {
        format!("{}h {}m", hours, remaining_mins)
    } else {
        format!("{}h", hours)
    }
}

fn format_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_cost(c: f64) -> String {
    if c <= 0.0 {
        return String::new();
    }
    if c < 0.001 {
        format!("${:.6}", c)
    } else if c < 0.01 {
        format!("${:.4}", c)
    } else {
        format!("${:.2}", c)
    }
}

fn format_relative_time(start_time_ns: i64) -> String {
    let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let diff_secs = (now_ns - start_time_ns) / 1_000_000_000;
    if diff_secs < 0 {
        return "just now".to_string();
    }
    let diff_secs = diff_secs as u64;
    if diff_secs < 60 {
        return "just now".to_string();
    }
    if diff_secs < 3600 {
        let mins = diff_secs / 60;
        return if mins == 1 {
            "1 min ago".to_string()
        } else {
            format!("{} mins ago", mins)
        };
    }
    if diff_secs < 86400 {
        let hours = diff_secs / 3600;
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        };
    }
    if diff_secs < 86400 * 30 {
        let days = diff_secs / 86400;
        return if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        };
    }
    if diff_secs < 86400 * 365 {
        let months = diff_secs / (86400 * 30);
        return if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        };
    }
    let years = diff_secs / (86400 * 365);
    if years == 1 {
        "1 year ago".to_string()
    } else {
        format!("{} years ago", years)
    }
}

fn term_width() -> usize {
    terminal::size().map(|(w, _)| w as usize).unwrap_or(100)
}

fn separator(width: usize) -> String {
    "─".repeat(width)
}

// ─────────────────────────────────────────────────────────────────────────────
// Trace list display
// ─────────────────────────────────────────────────────────────────────────────

pub async fn print_trace_list(client: &Distri, limit: i64) {
    match client.list_traces(Some(limit)).await {
        Ok(mut traces) => {
            if traces.is_empty() {
                println!("No traces found.");
                return;
            }
            // Sort oldest first so most recent appears at the bottom
            traces.sort_by_key(|t| t.start_time_ns);

            let width = term_width().min(90);
            println!();
            println!(
                "  {}{}Traces{} ({} found)",
                COLOR_BOLD,
                COLOR_BRIGHT_GREEN,
                COLOR_RESET,
                traces.len()
            );
            println!("  {}{}{}", COLOR_GRAY, separator(width - 2), COLOR_RESET);
            println!();

            for trace in &traces {
                print_trace_summary(trace, width);
            }
            println!("  {}{}{}\n", COLOR_GRAY, separator(width - 2), COLOR_RESET);
            println!(
                "  {}Use `distri traces show <trace-id>` to view details{}",
                COLOR_GRAY, COLOR_RESET
            );
            println!();
        }
        Err(e) => {
            eprintln!("Failed to list traces: {}", e);
        }
    }
}

fn print_trace_summary(trace: &TraceSummary, _width: usize) {
    let duration = format_duration_ns(trace.end_time_ns - trace.start_time_ns);
    let cost = format_cost(trace.total_cost);
    let tokens = if trace.input_tokens > 0 {
        format!("{}tokens", format_tokens(trace.input_tokens))
    } else {
        String::new()
    };
    let models: Vec<&str> = trace.models.iter().map(|m| m.as_str()).collect();
    let model_str = if models.is_empty() {
        String::new()
    } else {
        models.join(", ")
    };

    // Line 1: name + stats
    let trace_id_short = &trace.trace_id;

    let relative_time = format_relative_time(trace.start_time_ns);

    let spans_text = format!("{}{} spans{}", COLOR_GRAY, trace.span_count, COLOR_RESET);
    println!(
        "  {}{}{} {}  {}{}  {}{}  {}{}{}",
        COLOR_BOLD,
        trace.name,
        COLOR_RESET,
        spans_text,
        COLOR_BRIGHT_CYAN,
        duration,
        COLOR_BRIGHT_YELLOW,
        cost,
        COLOR_DIM,
        relative_time,
        COLOR_RESET,
    );

    // Line 2: trace ID, thread, tokens, models
    let thread_str = trace
        .thread_id
        .as_deref()
        .map(|t| {
            let short = if t.len() > 8 { &t[..8] } else { t };
            format!(" · {}", short)
        })
        .unwrap_or_default();
    println!(
        "  {}{}{}{}  {}{}  {}{}{}",
        COLOR_GRAY,
        trace_id_short,
        thread_str,
        COLOR_RESET,
        COLOR_GRAY,
        tokens,
        COLOR_DIM,
        model_str,
        COLOR_RESET,
    );

    // Line 3: input preview (if any)
    if let Some(ref preview) = trace.input_preview {
        let trimmed = preview.trim();
        if !trimmed.is_empty() {
            let display = if trimmed.len() > 80 {
                format!("{}...", &trimmed[..80])
            } else {
                trimmed.to_string()
            };
            println!("  {}{}{}", COLOR_DIM, display, COLOR_RESET);
        }
    }

    println!();
}

// ─────────────────────────────────────────────────────────────────────────────
// Trace detail (Gantt chart)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn print_trace_detail(
    client: &Distri,
    id: &str,
    span_filter: Option<&str>,
    verbose: bool,
) {
    // Try as trace_id first, then as thread_id
    let otlp = match client.get_spans(Some(id), None).await {
        Ok(v) => {
            // Check if we got spans
            let has_spans = v
                .get("resourceSpans")
                .and_then(|rs| rs.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if has_spans {
                v
            } else {
                // Try as thread_id
                match client.get_spans(None, Some(id)).await {
                    Ok(v2) => v2,
                    Err(_) => v, // Use original empty response
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to get spans: {}", e);
            return;
        }
    };

    let roots = parse_otlp_spans(&otlp);
    if roots.is_empty() {
        println!("No spans found for '{}'.", id);
        return;
    }

    // Find time range across all spans
    let (min_start, max_end) = find_time_range(&roots);
    let total_span_count = count_spans(&roots);

    let width = term_width().min(120);
    let bar_width = width.saturating_sub(40).max(20);

    // Header
    let root_name = roots.first().map(|r| r.name.as_str()).unwrap_or("trace");
    let root_duration = format_duration_ns(max_end - min_start);

    // Collect aggregate stats
    let (total_tokens, total_cost) = aggregate_stats(&roots);

    let id_short = if id.len() > 16 { &id[..16] } else { id };

    println!();
    println!(
        "  {}{}Trace: {}{} {}({}){}\n  {}{} spans · {} · {}tokens · {}{}",
        COLOR_BOLD,
        COLOR_BRIGHT_GREEN,
        root_name,
        COLOR_RESET,
        COLOR_GRAY,
        id_short,
        COLOR_RESET,
        COLOR_GRAY,
        total_span_count,
        root_duration,
        format_tokens(total_tokens),
        format_cost(total_cost),
        COLOR_RESET,
    );
    println!("  {}{}{}", COLOR_GRAY, separator(width - 2), COLOR_RESET);
    println!();

    // Render Gantt chart
    for root in &roots {
        render_span_row(root, 0, min_start, max_end, bar_width, span_filter, verbose);
    }

    println!("  {}{}{}", COLOR_GRAY, separator(width - 2), COLOR_RESET);

    // Print input/output for root span
    if let Some(root) = roots.first() {
        let input = root.input_value();
        let output = root.output_value();

        if input.is_some() || output.is_some() {
            println!();
        }

        if let Some(inp) = input {
            println!("  {}{}INPUT:{}", COLOR_BOLD, COLOR_BRIGHT_BLUE, COLOR_RESET);
            if verbose {
                let display = format_value_pretty(inp);
                for line in display.lines() {
                    println!("  {}{}{}", COLOR_DIM, line, COLOR_RESET);
                }
            } else {
                let summary = extract_readable_summary(inp, 200);
                println!("  {}{}{}", COLOR_DIM, summary, COLOR_RESET);
            }
            println!();
        }

        if let Some(out) = output {
            println!(
                "  {}{}OUTPUT:{}",
                COLOR_BOLD, COLOR_BRIGHT_GREEN, COLOR_RESET
            );
            if verbose {
                let display = format_value_pretty(out);
                for line in display.lines() {
                    println!("  {}{}{}", COLOR_DIM, line, COLOR_RESET);
                }
            } else {
                let summary = extract_readable_summary(out, 200);
                println!("  {}{}{}", COLOR_DIM, summary, COLOR_RESET);
            }
            println!();
        }
    }
}

fn render_span_row(
    span: &CliSpan,
    depth: usize,
    min_start: i64,
    max_end: i64,
    bar_width: usize,
    span_filter: Option<&str>,
    verbose: bool,
) {
    // If filtering, check if this span matches
    if let Some(filter) = span_filter {
        let filter_lower = filter.to_lowercase();
        let matches = span.span_id.to_lowercase().contains(&filter_lower)
            || span.name.to_lowercase().contains(&filter_lower)
            || span.clean_title().to_lowercase().contains(&filter_lower);
        if !matches {
            // Still recurse into children
            for child in &span.children {
                render_span_row(
                    child,
                    depth,
                    min_start,
                    max_end,
                    bar_width,
                    span_filter,
                    verbose,
                );
            }
            return;
        }
    }

    let cat = span.category();
    let indent = "  ".repeat(depth + 1);
    let title = span.clean_title();
    let duration = format_duration_ns(span.duration_ns());

    // Build info parts
    let mut info_parts: Vec<String> = Vec::new();

    if let Some(tokens) = span.input_tokens() {
        if tokens > 0 {
            info_parts.push(format!(
                "{}{}↑{}",
                COLOR_GRAY,
                format_tokens(tokens),
                COLOR_RESET
            ));
        }
    }
    if let Some(cost) = span.cost() {
        if cost > 0.0 {
            info_parts.push(format!(
                "{}{}{}",
                COLOR_BRIGHT_YELLOW,
                format_cost(cost),
                COLOR_RESET
            ));
        }
    }

    let info_str = if info_parts.is_empty() {
        String::new()
    } else {
        format!("  {}", info_parts.join("  "))
    };

    let span_id_display = &span.span_id;

    // Category badge
    let badge = format!("{}[{}]{}", cat.color(), cat.label(), COLOR_RESET);

    // Span row: indent + badge + title + span_id + info
    println!(
        "{}{} {}{}{}  {}{}{}  {}",
        indent,
        badge,
        COLOR_BOLD,
        title,
        COLOR_RESET,
        COLOR_GRAY,
        span_id_display,
        COLOR_RESET,
        info_str,
    );

    // Show input/output: short summary by default, full with -v
    let extra_indent = "  ".repeat(depth + 2);
    if verbose {
        if let Some(inp) = span.input_value() {
            let formatted = format_value_pretty(inp);
            if !formatted.is_empty() {
                println!("{}{}→ in:{}", extra_indent, COLOR_DIM, COLOR_RESET);
                for line in formatted.lines() {
                    println!("{}{}{}{}", extra_indent, COLOR_DIM, line, COLOR_RESET);
                }
            }
        }
        if let Some(out) = span.output_value() {
            let formatted = format_value_pretty(out);
            if !formatted.is_empty() {
                println!("{}{}← out:{}", extra_indent, COLOR_DIM, COLOR_RESET);
                for line in formatted.lines() {
                    println!("{}{}{}{}", extra_indent, COLOR_DIM, line, COLOR_RESET);
                }
            }
        }
    } else {
        if let Some(inp) = span.input_value() {
            let summary = extract_readable_summary(inp, 100);
            if !summary.is_empty() {
                println!("{}{}→ {}{}", extra_indent, COLOR_DIM, summary, COLOR_RESET);
            }
        }
        if let Some(out) = span.output_value() {
            let summary = extract_readable_summary(out, 100);
            if !summary.is_empty() {
                println!("{}{}← {}{}", extra_indent, COLOR_DIM, summary, COLOR_RESET);
            }
        }
    }

    // Right-align duration on same conceptual line - print on next sub-line with bar
    // Calculate timeline bar
    let total_range = max_end - min_start;
    if total_range > 0 {
        let start_pct = (span.start_time_ns - min_start) as f64 / total_range as f64;
        let width_pct = span.duration_ns() as f64 / total_range as f64;

        let bar_start = (start_pct * bar_width as f64) as usize;
        let bar_len = (width_pct * bar_width as f64).max(1.0) as usize;
        let bar_end = bar_width.saturating_sub(bar_start + bar_len);

        let empty_char = "░";
        let fill_char = cat.bar_char();

        let bar = format!(
            "{}{}{}{}{}{}",
            COLOR_DIM,
            empty_char.repeat(bar_start),
            cat.color(),
            fill_char.repeat(bar_len),
            COLOR_DIM,
            empty_char.repeat(bar_end),
        );

        let bar_indent = "  ".repeat(depth + 1);
        let badge_space = " ".repeat(cat.label().len() + 3); // [XXX] + space
        println!(
            "{}{}{}{} {}{}{}",
            bar_indent, badge_space, bar, COLOR_RESET, COLOR_BRIGHT_CYAN, duration, COLOR_RESET,
        );
    }

    // Recurse into children
    for child in &span.children {
        render_span_row(
            child,
            depth + 1,
            min_start,
            max_end,
            bar_width,
            span_filter,
            verbose,
        );
    }
}

fn find_time_range(spans: &[CliSpan]) -> (i64, i64) {
    let mut min_start = i64::MAX;
    let mut max_end = i64::MIN;

    fn walk(span: &CliSpan, min: &mut i64, max: &mut i64) {
        if span.start_time_ns < *min {
            *min = span.start_time_ns;
        }
        if span.end_time_ns > *max {
            *max = span.end_time_ns;
        }
        for child in &span.children {
            walk(child, min, max);
        }
    }

    for span in spans {
        walk(span, &mut min_start, &mut max_end);
    }

    if min_start == i64::MAX {
        (0, 1)
    } else {
        (min_start, max_end)
    }
}

fn count_spans(spans: &[CliSpan]) -> usize {
    fn walk(span: &CliSpan) -> usize {
        1 + span.children.iter().map(walk).sum::<usize>()
    }
    spans.iter().map(walk).sum()
}

fn aggregate_stats(spans: &[CliSpan]) -> (i64, f64) {
    fn walk(span: &CliSpan, tokens: &mut i64, cost: &mut f64) {
        if let Some(t) = span.input_tokens() {
            *tokens += t;
        }
        if let Some(c) = span.cost() {
            *cost += c;
        }
        for child in &span.children {
            walk(child, tokens, cost);
        }
    }

    let mut tokens = 0i64;
    let mut cost = 0.0f64;
    for span in spans {
        walk(span, &mut tokens, &mut cost);
    }
    (tokens, cost)
}

/// Extract a human-readable one-line summary from a value.
/// If it's JSON, pull out the first text/content field. Otherwise treat as plain text.
fn extract_readable_summary(text: &str, max_len: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Try parsing as JSON and extract something readable
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(s) = extract_text_from_json(&val) {
            let oneline = s.lines().next().unwrap_or("").trim().to_string();
            if oneline.len() > max_len {
                return format!("{}...", &oneline[..max_len]);
            }
            return oneline;
        }
    }

    // Plain text: take first non-empty line
    let line = trimmed
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.len() > max_len {
        format!("{}...", &line[..max_len])
    } else {
        line.to_string()
    }
}

/// Walk JSON to find the first human-readable text string.
fn extract_text_from_json(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                // If it's nested JSON, recurse
                if let Ok(inner) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    return extract_text_from_json(&inner);
                }
                return Some(trimmed.to_string());
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                // Look for message-like objects with content/text/parts
                if let Some(obj) = item.as_object() {
                    // Skip system messages, prefer user/assistant
                    let role = obj.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    if role == "system" {
                        continue;
                    }
                    // Try common content fields
                    for key in &["content", "text", "parts", "value"] {
                        if let Some(v) = obj.get(*key) {
                            if let Some(s) = extract_text_from_json(v) {
                                return Some(s);
                            }
                        }
                    }
                }
                if let Some(s) = extract_text_from_json(item) {
                    return Some(s);
                }
            }
            None
        }
        serde_json::Value::Object(obj) => {
            // Try common content fields first
            for key in &[
                "content", "text", "message", "output", "input", "value", "parts",
            ] {
                if let Some(v) = obj.get(*key) {
                    if let Some(s) = extract_text_from_json(v) {
                        return Some(s);
                    }
                }
            }
            // Fall back to any string field
            for (_, v) in obj {
                if let Some(s) = extract_text_from_json(v) {
                    return Some(s);
                }
            }
            None
        }
        _ => None,
    }
}

/// Format a value for verbose output. Pretty-print JSON, show text as-is.
fn format_value_pretty(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Try JSON pretty-print
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // If it's a string containing JSON, unwrap one level
        if let serde_json::Value::String(inner) = &val {
            if let Ok(inner_val) = serde_json::from_str::<serde_json::Value>(inner.trim()) {
                return serde_json::to_string_pretty(&inner_val).unwrap_or_else(|_| inner.clone());
            }
            return inner.clone();
        }
        return serde_json::to_string_pretty(&val).unwrap_or_else(|_| trimmed.to_string());
    }

    trimmed.to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Command handler
// ─────────────────────────────────────────────────────────────────────────────

pub async fn handle_traces_command(client: &Distri, command: TracesCommands) -> Result<()> {
    match command {
        TracesCommands::List { limit } => {
            print_trace_list(client, limit).await;
        }
        TracesCommands::Show {
            id,
            latest,
            span,
            verbose,
        } => {
            let resolved_id = if latest {
                match resolve_latest_trace_id(client).await {
                    Some(id) => id,
                    None => return Ok(()),
                }
            } else if let Some(id) = id {
                id
            } else {
                eprintln!("Error: provide a trace ID or use --latest");
                return Ok(());
            };
            print_trace_detail(client, &resolved_id, span.as_deref(), verbose).await;
        }
        TracesCommands::Export {
            trace_id,
            latest,
            output,
        } => {
            let resolved_id = if latest {
                match resolve_latest_trace_id(client).await {
                    Some(id) => id,
                    None => return Ok(()),
                }
            } else if let Some(id) = trace_id {
                id
            } else {
                eprintln!("Error: provide a trace ID or use --latest");
                return Ok(());
            };
            export_trace_fixture(client, &resolved_id, output.as_deref()).await?;
        }
    }
    Ok(())
}

async fn resolve_latest_trace_id(client: &Distri) -> Option<String> {
    match client.list_traces(Some(1)).await {
        Ok(traces) => {
            if traces.is_empty() {
                eprintln!("No traces found.");
                None
            } else {
                // The API returns most recent first with limit=1
                let latest = traces.iter().max_by_key(|t| t.start_time_ns).unwrap();
                Some(latest.trace_id.clone())
            }
        }
        Err(e) => {
            eprintln!("Failed to list traces: {}", e);
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Export trace as replay fixture
// ─────────────────────────────────────────────────────────────────────────────

/// Export a trace's LLM call pairs as a JSON replay fixture.
///
/// The fixture file can be loaded by `TraceReplayExecutor` in distri-core
/// for deterministic integration testing.
async fn export_trace_fixture(
    client: &Distri,
    trace_id: &str,
    output_path: Option<&str>,
) -> Result<()> {
    eprintln!("Fetching spans for trace {}...", trace_id);

    let otlp = match client.get_spans(Some(trace_id), None).await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to fetch spans: {}", e);
            return Ok(());
        }
    };

    // Parse OTLP spans into flat list
    let flat_spans = parse_otlp_to_flat_json(&otlp);

    if flat_spans.is_empty() {
        eprintln!("No spans found for trace {}", trace_id);
        return Ok(());
    }

    // Filter to LLM spans and build fixture
    let fixture = build_export_fixture(trace_id, &flat_spans);

    eprintln!(
        "Exported {} LLM call(s) from {} total span(s)",
        fixture.calls.len(),
        flat_spans.len()
    );

    let json = serde_json::to_string_pretty(&fixture)
        .map_err(|e| anyhow::anyhow!("Failed to serialize fixture: {}", e))?;

    match output_path {
        Some(path) => {
            std::fs::write(path, &json)
                .map_err(|e| anyhow::anyhow!("Failed to write {}: {}", path, e))?;
            eprintln!("Written to {}", path);
        }
        None => {
            println!("{}", json);
        }
    }

    Ok(())
}

/// Fixture types for export (matches distri-core TraceFixture format)
#[derive(serde::Serialize)]
struct ExportFixture {
    id: String,
    description: Option<String>,
    agent_id: Option<String>,
    calls: Vec<ExportLLMCall>,
    metadata: serde_json::Value,
}

#[derive(serde::Serialize)]
struct ExportLLMCall {
    call_index: usize,
    model: Option<String>,
    input: serde_json::Value,
    output_content: String,
    tool_calls: Vec<ExportToolCall>,
    finish_reason: String,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

#[derive(serde::Serialize)]
struct ExportToolCall {
    tool_call_id: String,
    tool_name: String,
    input: serde_json::Value,
}

/// Parse OTLP JSON into flat span objects with attributes as key-value maps.
fn parse_otlp_to_flat_json(otlp: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut result = Vec::new();

    let resource_spans = match otlp.get("resourceSpans").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return result,
    };

    for rs in resource_spans {
        let scope_spans = match rs.get("scopeSpans").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };
        for ss in scope_spans {
            let spans = match ss.get("spans").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for span_obj in spans {
                // Convert OTLP attributes array to flat map
                let mut attrs_map = serde_json::Map::new();
                if let Some(attrs) = span_obj.get("attributes").and_then(|v| v.as_array()) {
                    for attr in attrs {
                        if let (Some(key), Some(value)) =
                            (attr.get("key").and_then(|k| k.as_str()), attr.get("value"))
                        {
                            let val_str = extract_otlp_value(value);
                            attrs_map.insert(key.to_string(), serde_json::Value::String(val_str));
                        }
                    }
                }

                let mut flat = serde_json::Map::new();
                if let Some(id) = span_obj.get("spanId") {
                    flat.insert("span_id".to_string(), id.clone());
                }
                if let Some(name) = span_obj.get("name") {
                    flat.insert("name".to_string(), name.clone());
                }
                if let Some(start) = span_obj.get("startTimeUnixNano") {
                    let ns = start
                        .as_str()
                        .and_then(|s| s.parse::<i64>().ok())
                        .or_else(|| start.as_i64())
                        .unwrap_or(0);
                    flat.insert(
                        "start_time_ns".to_string(),
                        serde_json::Value::Number(ns.into()),
                    );
                }
                flat.insert(
                    "attributes".to_string(),
                    serde_json::Value::Object(attrs_map),
                );
                result.push(serde_json::Value::Object(flat));
            }
        }
    }

    result
}

/// Build an export fixture from flat span objects.
fn build_export_fixture(trace_id: &str, spans: &[serde_json::Value]) -> ExportFixture {
    let mut llm_spans: Vec<&serde_json::Value> =
        spans.iter().filter(|span| is_llm_span(span)).collect();

    llm_spans.sort_by_key(|span| {
        span.get("start_time_ns")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    });

    let agent_id = spans
        .first()
        .and_then(|s| s.get("attributes"))
        .and_then(|a| a.get("distri.agent.id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let calls = llm_spans
        .iter()
        .enumerate()
        .map(|(idx, span)| {
            let attrs = span.get("attributes").cloned().unwrap_or_default();

            let input = attrs
                .get("input.value")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .unwrap_or(serde_json::Value::Null);

            let output_raw = attrs
                .get("output.value")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let (output_content, tool_calls, finish_reason) = parse_llm_output(output_raw);

            let model = attrs
                .get("gen_ai.request.model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let input_tokens = attrs
                .get("gen_ai.usage.input_tokens")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok());

            let output_tokens = attrs
                .get("gen_ai.usage.output_tokens")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok());

            ExportLLMCall {
                call_index: idx,
                model,
                input,
                output_content,
                tool_calls,
                finish_reason,
                input_tokens,
                output_tokens,
            }
        })
        .collect();

    ExportFixture {
        id: trace_id.to_string(),
        description: None,
        agent_id,
        calls,
        metadata: serde_json::json!({"trace_id": trace_id}),
    }
}

fn is_llm_span(span: &serde_json::Value) -> bool {
    let attrs = match span.get("attributes") {
        Some(a) => a,
        None => return false,
    };

    if let Some(kind) = attrs
        .get("openinference.span.kind")
        .and_then(|v| v.as_str())
    {
        if kind.eq_ignore_ascii_case("LLM") {
            return true;
        }
    }

    if let Some(op) = attrs.get("gen_ai.operation.name").and_then(|v| v.as_str()) {
        if op == "chat" || op == "completion" {
            return true;
        }
    }

    if attrs.get("gen_ai.request.model").is_some() && attrs.get("input.value").is_some() {
        return true;
    }

    false
}

/// Parse an LLM span's `output.value` (a serialized assistant [`Message`] in
/// Distri wire format) into text content + tool calls (with `tool_call_id`).
fn parse_llm_output(output_raw: &str) -> (String, Vec<ExportToolCall>, String) {
    if let Ok(message) = serde_json::from_str::<Message>(output_raw) {
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for part in &message.parts {
            match part {
                Part::Text(text) => content.push_str(text),
                Part::ToolCall(tc) => tool_calls.push(ExportToolCall {
                    tool_call_id: tc.tool_call_id.clone(),
                    tool_name: tc.tool_name.clone(),
                    input: tc.input.clone(),
                }),
                _ => {}
            }
        }
        let finish_reason = if tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        };
        return (content, tool_calls, finish_reason.to_string());
    }

    // Fallback: plain-text output that isn't a serialized Message.
    (output_raw.to_string(), vec![], "stop".to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Optimize command handler
// ─────────────────────────────────────────────────────────────────────────────

pub async fn handle_optimize_command(client: &Distri, command: OptimizeCommands) -> Result<()> {
    match command {
        OptimizeCommands::Analyze {
            agent,
            lookback,
            format,
        } => {
            handle_optimize_analyze(client, agent.as_deref(), lookback, &format).await?;
        }
        OptimizeCommands::Suggest { agent, target } => {
            handle_optimize_suggest(client, agent.as_deref(), target.as_deref()).await?;
        }
        OptimizeCommands::Loop {
            iterations,
            agent,
            dry_run,
        } => {
            handle_optimize_loop(client, iterations, agent.as_deref(), dry_run).await?;
        }
    }
    Ok(())
}

async fn handle_optimize_analyze(
    client: &Distri,
    agent: Option<&str>,
    lookback: i64,
    format: &str,
) -> Result<()> {
    eprintln!(
        "Analyzing {} recent traces{}...",
        lookback,
        agent
            .map(|a| format!(" for agent '{}'", a))
            .unwrap_or_default()
    );

    let traces = client.list_traces(Some(lookback)).await?;

    if traces.is_empty() {
        eprintln!("No traces found.");
        return Ok(());
    }

    // Analyze each trace
    let mut total_input_tokens = 0i64;
    let mut total_cost = 0.0f64;
    let _error_count = 0usize;
    let _tool_freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut model_freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for trace in &traces {
        total_input_tokens += trace.input_tokens;
        total_cost += trace.total_cost;

        for model in trace.models.iter() {
            *model_freq.entry(model.clone()).or_default() += 1;
        }
    }

    let avg_tokens = total_input_tokens / traces.len().max(1) as i64;
    let avg_cost = total_cost / traces.len().max(1) as f64;

    if format == "json" {
        let report = serde_json::json!({
            "total_traces": traces.len(),
            "avg_input_tokens": avg_tokens,
            "avg_cost": avg_cost,
            "total_cost": total_cost,
            "model_usage": model_freq,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "\n{}── Trace Analysis Report ──{}",
            COLOR_BRIGHT_GREEN, COLOR_RESET
        );
        println!("  Traces analyzed:    {}", traces.len());
        println!("  Avg input tokens:   {}", avg_tokens);
        println!("  Avg cost:           ${:.4}", avg_cost);
        println!("  Total cost:         ${:.4}", total_cost);
        println!();

        if !model_freq.is_empty() {
            println!("  {}Models used:{}", COLOR_BRIGHT_GREEN, COLOR_RESET);
            let mut sorted: Vec<_> = model_freq.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (model, count) in sorted {
                println!("    {} — {} traces", model, count);
            }
        }
    }

    Ok(())
}

async fn handle_optimize_suggest(
    _client: &Distri,
    _agent: Option<&str>,
    _target: Option<&str>,
) -> Result<()> {
    eprintln!(
        "{}Suggest{}: analyzing traces and generating suggestions...",
        COLOR_BRIGHT_GREEN, COLOR_RESET
    );
    eprintln!();
    eprintln!("Note: Full suggestion generation requires the trace analysis service");
    eprintln!("running in distri-cloud. Use `distri optimize analyze` to view");
    eprintln!("current trace patterns first.");
    Ok(())
}

async fn handle_optimize_loop(
    _client: &Distri,
    iterations: usize,
    _agent: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    eprintln!(
        "{}Optimization loop{}: {} iterations{}",
        COLOR_BRIGHT_GREEN,
        COLOR_RESET,
        iterations,
        if dry_run { " (dry run)" } else { "" }
    );
    eprintln!();
    eprintln!("Note: The optimization loop requires the trace analysis and optimization");
    eprintln!("services running in distri-cloud. The full loop flow is:");
    eprintln!("  1. Analyze recent traces → identify weak scenarios");
    eprintln!("  2. Select target skill via affinity map");
    eprintln!("  3. Generate mutation via LLM");
    eprintln!("  4. Evaluate mutation against scenarios");
    eprintln!("  5. Keep/discard based on score delta");
    eprintln!();
    eprintln!("Use `distri optimize analyze` to start with trace analysis.");
    Ok(())
}
