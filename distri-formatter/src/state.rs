//! Shared event-processing state types extracted from `distri/src/printer.rs`.

use std::collections::HashMap;

use distri_types::MessageRole;

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
}

#[derive(Debug, Clone)]
pub struct StepState {
    pub id: String,
    pub title: String,
    pub index: usize,
    pub status: String,
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

/// Tracks the state of an ongoing agent conversation.
#[derive(Debug, Default)]
pub struct ChatState {
    pub messages: HashMap<String, MessageState>,
    pub steps: HashMap<String, StepState>,
    pub tool_calls: HashMap<String, ToolCallState>,
    pub current_message_id: Option<String>,
    pub is_planning: bool,
    pub printed_header: bool,
    pub current_agent: Option<String>,
    pub thread_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure-logic helpers (no terminal dependencies)
// ---------------------------------------------------------------------------

/// Returns `true` if this tool call looks like an internal discovery/probe call
/// that shouldn't be shown to the user.
pub fn is_probe_call(name: &str, input: &serde_json::Value) -> bool {
    match name {
        // Final/reflect/console_log are internal — their output goes through
        // item.message, not through the event stream.
        "final" | "reflect" | "console_log" => true,
        "load_skill" => {
            let skill = input
                .get("skill_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            skill == "?" || skill.is_empty()
        }
        "distri_request" => {
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

/// Format a tool call into a human-readable one-liner, e.g. `load_skill("my_skill")`.
pub fn format_tool_call(name: &str, input: &serde_json::Value) -> String {
    let str_field = |key: &str| {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let truncate = |s: &str, max: usize| -> String {
        if s.len() <= max {
            return s.to_string();
        }
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    };

    match name {
        // Claude Code-style local tools
        "Bash" => {
            let cmd = str_field("command");
            // Show first line, truncated
            let first_line = cmd.lines().next().unwrap_or(&cmd);
            format!("Bash(\"{}\")", truncate(first_line, 80))
        }
        "Read" => {
            let path = str_field("file_path");
            let suffix = match (
                input.get("offset").and_then(|v| v.as_u64()),
                input.get("limit").and_then(|v| v.as_u64()),
            ) {
                (Some(off), Some(lim)) => format!(", lines {}-{}", off + 1, off + lim),
                (Some(off), None) => format!(", from line {}", off + 1),
                _ => String::new(),
            };
            format!("Read(\"{}\"{})", truncate(&path, 60), suffix)
        }
        "Write" => {
            let path = str_field("file_path");
            let lines = input
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.lines().count())
                .unwrap_or(0);
            format!("Write(\"{}\", {} lines)", truncate(&path, 60), lines)
        }
        "Edit" => {
            let path = str_field("file_path");
            let replace_all = input
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if replace_all {
                format!("Edit(\"{}\", replace_all)", truncate(&path, 60))
            } else {
                format!("Edit(\"{}\")", truncate(&path, 60))
            }
        }
        "Glob" => {
            let pattern = str_field("pattern");
            match input.get("path").and_then(|v| v.as_str()) {
                Some(p) if !p.is_empty() => {
                    format!("Glob(\"{}\", path: \"{}\")", pattern, truncate(p, 40))
                }
                _ => format!("Glob(\"{}\")", pattern),
            }
        }
        "Grep" => {
            let pattern = str_field("pattern");
            match input.get("path").and_then(|v| v.as_str()) {
                Some(p) if !p.is_empty() => {
                    format!("Grep(\"{}\", path: \"{}\")", truncate(&pattern, 40), truncate(p, 40))
                }
                _ => format!("Grep(\"{}\")", truncate(&pattern, 60)),
            }
        }
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
        "start_shell" | "stop_shell" => format!("{}(...)", name),
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
        "final" | "reflect" | "console_log" => format!("{}(...)", name),
        _ => {
            // HTTP factory tools (e.g. distri_request, zippy_request) use
            // {path, method, body?} input — print as "name(METHOD /path)".
            if let (Some(path), Some(method)) = (
                input.get("path").and_then(|v| v.as_str()),
                input.get("method").and_then(|v| v.as_str()),
            ) {
                return format!("{}({} {})", name, method, truncate(path, 60));
            }
            let compact = serde_json::to_string(input).unwrap_or_else(|_| "...".into());
            let preview = truncate(&compact, 80);
            format!("{}({})", name, preview)
        }
    }
}

/// Compact JSON representation of a tool input, or `"..."` on failure / empty object.
pub fn format_tool_input(input: &serde_json::Value) -> String {
    if input.is_object() && input.as_object().map(|m| m.is_empty()).unwrap_or(false) {
        return "...".into();
    }
    serde_json::to_string(input).unwrap_or_else(|_| "...".into())
}
