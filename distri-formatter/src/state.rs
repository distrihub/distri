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
            let method = input.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
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
            // Show first line, truncated. Escape embedded `"` so a command
            // like `python3 -c "print('hi')"` doesn't render with an
            // unbalanced quote after truncation cuts the inner string. The
            // rendered form is for display only; not meant to be a faithful
            // shell-escape, just a visually clean preview.
            let first_line = cmd.lines().next().unwrap_or(&cmd);
            let escaped = first_line.replace('"', "\\\"");
            format!("Bash(\"{}\")", truncate(&escaped, 80))
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
                    format!(
                        "Grep(\"{}\", path: \"{}\")",
                        truncate(&pattern, 40),
                        truncate(p, 40)
                    )
                }
                _ => format!("Grep(\"{}\")", truncate(&pattern, 60)),
            }
        }
        "load_skill" => format!("load_skill(\"{}\")", str_field("skill_name")),
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
            format!("inject_connection_env(\"{}\")", str_field("provider_name"))
        }
        "transfer_to_agent" => {
            format!("transfer_to_agent(\"{}\")", str_field("agent_name"))
        }
        "call_agent" => {
            // Universal agent dispatch. Prefer (named agent, mode) over the
            // raw JSON dump — matches the rendering style of every other
            // arm. Ad-hoc agents (no `agent`, but `system_prompt` set) get
            // a "ad-hoc" placeholder.
            let agent = input
                .get("agent")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    if input.get("system_prompt").is_some() {
                        "<ad-hoc>".to_string()
                    } else {
                        "?".to_string()
                    }
                });
            let mode = input
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("in_process");
            format!("call_agent(\"{}\", mode: {})", truncate(&agent, 40), mode)
        }
        "run_skill" => {
            // `run_skill` previously fell through to the default JSON-dump
            // arm, which produced unreadable lines like
            // `run_skill({"mode":"fork","prompt":"…truncated…)` in chat
            // surfaces. Mirror `call_agent`'s style — show what the user
            // actually cares about: which skill, what mode.
            let skill = str_field("skill_id");
            let mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("fork");
            format!("run_skill(\"{}\", mode: {})", truncate(&skill, 40), mode)
        }
        "final" | "reflect" | "console_log" => format!("{}(...)", name),
        _ => {
            // HTTP factory tools (e.g. distri_request) take
            // {path | url, method?, body?, headers?}. `path` is for the
            // distri platform API (base_url is prepended); `url` is for
            // external APIs (absolute, base_url skipped). Either flavor is
            // an HTTP-factory call. `method` defaults to GET.
            let path_or_url = input
                .get("url")
                .and_then(|v| v.as_str())
                .or_else(|| input.get("path").and_then(|v| v.as_str()));
            if let Some(target) = path_or_url {
                let method = input
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET");
                let connection = input
                    .get("headers")
                    .and_then(|h| h.get("x-connection-id"))
                    .and_then(|v| v.as_str())
                    .map(render_connection_label_short);
                if let Some(conn) = connection {
                    return format!("{}({} → {} {})", name, conn, method, truncate(target, 50));
                }
                return format!("{}({} {})", name, method, truncate(target, 60));
            }
            let compact = serde_json::to_string(input).unwrap_or_else(|_| "...".into());
            let preview = truncate(&compact, 80);
            format!("{}({})", name, preview)
        }
    }
}

/// Compact connection-id label for inline tool-call lines (the one-liner
/// path used by every surface). UUIDs become "🔐"; named connections
/// (e.g. "google-calendar") render as-is.
fn render_connection_label_short(conn_id: &str) -> String {
    if looks_like_uuid(conn_id) {
        "🔐".to_string()
    } else {
        conn_id.to_string()
    }
}

/// True when `s` matches the canonical UUID 8-4-4-4-12 hex shape (any
/// version, hyphenated). Avoids pulling in the `uuid` crate just for this.
fn looks_like_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        let is_hyphen_pos = matches!(i, 8 | 13 | 18 | 23);
        if is_hyphen_pos {
            if b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Compact JSON representation of a tool input, or `"..."` on failure / empty object.
pub fn format_tool_input(input: &serde_json::Value) -> String {
    if input.is_object() && input.as_object().map(|m| m.is_empty()).unwrap_or(false) {
        return "...".into();
    }
    serde_json::to_string(input).unwrap_or_else(|_| "...".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn call_agent_named_with_mode() {
        let input = json!({"agent": "distri_runner", "mode": "fork", "prompt": "hi"});
        assert_eq!(
            format_tool_call("call_agent", &input),
            "call_agent(\"distri_runner\", mode: fork)"
        );
    }

    #[test]
    fn call_agent_default_mode_when_missing() {
        let input = json!({"agent": "coder", "prompt": "hi"});
        assert_eq!(
            format_tool_call("call_agent", &input),
            "call_agent(\"coder\", mode: in_process)"
        );
    }

    #[test]
    fn call_agent_ad_hoc() {
        let input = json!({"system_prompt": "you are a helpful assistant", "prompt": "hi"});
        assert_eq!(
            format_tool_call("call_agent", &input),
            "call_agent(\"<ad-hoc>\", mode: in_process)"
        );
    }

    #[test]
    fn http_factory_with_named_connection_shows_connection_first() {
        let input = json!({
            "method": "GET",
            "path": "/calendar/events",
            "headers": {"x-connection-id": "google-calendar"},
        });
        assert_eq!(
            format_tool_call("distri_request", &input),
            "distri_request(google-calendar → GET /calendar/events)"
        );
    }

    #[test]
    fn http_factory_with_uuid_connection_collapses_to_lock() {
        let input = json!({
            "method": "GET",
            "path": "/connections",
            "headers": {"x-connection-id": "f9ef3fe3-9203-422c-96d9-b36c4aa10c6d"},
        });
        assert_eq!(
            format_tool_call("distri_request", &input),
            "distri_request(🔐 → GET /connections)"
        );
    }

    #[test]
    fn http_factory_url_form_works_too() {
        let input = json!({
            "method": "GET",
            "url": "https://www.googleapis.com/calendar/v3/events",
        });
        let result = format_tool_call("distri_request", &input);
        assert!(
            result.contains("GET"),
            "should detect HTTP factory via url field: {result}"
        );
        assert!(result.contains("googleapis.com"));
    }

    #[test]
    fn http_factory_method_defaults_to_get() {
        let input = json!({"path": "/agents"});
        assert_eq!(
            format_tool_call("distri_request", &input),
            "distri_request(GET /agents)"
        );
    }

    #[test]
    fn http_factory_without_connection_shows_method_path() {
        let input = json!({"method": "POST", "path": "/v1/skills"});
        assert_eq!(
            format_tool_call("distri_request", &input),
            "distri_request(POST /v1/skills)"
        );
    }
}
