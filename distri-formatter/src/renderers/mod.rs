use std::io::Write;

/// `println!` into the renderer's sink. Write errors are dropped: a closed
/// pipe or a full buffer must never fail rendering.
macro_rules! outln {
    ($out:expr) => {{
        let _ = std::io::Write::write_all(&mut *$out, b"\n");
    }};
    ($out:expr, $($arg:tt)*) => {{
        let _ = std::io::Write::write_fmt(&mut *$out, format_args!("{}\n", format_args!($($arg)*)));
    }};
}

mod browser;
mod code;
pub(crate) mod data;
mod local_tools;
mod platform;
mod search;
mod shell;
pub mod tool_result;

pub use tool_result::render_tool_result;

use crate::colors::{COLOR_GRAY, COLOR_RED, COLOR_RESET};
use distri_types::ToolResponse;

/// Result line prefix: `  ⎿  ` (matches Claude Code style)
pub const RESULT_PREFIX: &str = "  ⎿  ";

/// Truncate a string to at most `max_bytes` bytes without splitting multi-byte characters.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Dispatch tool result rendering to the appropriate tool-specific renderer.
pub fn render_tool_output(out: &mut dyn Write, result: &ToolResponse, verbose: bool) {
    let _ = verbose; // verbose handled by caller; formatting is always the same

    let name = result.tool_name.as_str();

    match name {
        // Simple tools — suppress output (streamed text is enough)
        "final" | "reflect" | "console_log" | "transfer_to_agent" => {}

        // Platform / discovery tools
        "tool_search"
        | "load_skill"
        | "list_agents"
        | "list_skills"
        | "create_skill"
        | "delete_skill"
        | "write_to_storage"
        | "read_from_storage"
        | "inject_connection_env" => {
            platform::render_platform_tool(out, result);
        }

        // Browser / scraping
        "browsr_scrape" | "browsr_crawl" => browser::render_scrape(out, result),
        "browsr_browser" | "browser_step" => browser::render_browser_step(out, result),

        // Search
        "search" => search::render_search(out, result),

        // Local CLI tools (claude-code style)
        "Bash" | "Read" | "Write" | "Edit" | "Glob" | "Grep" => {
            local_tools::render_local_tool(out, result)
        }

        // Shell
        "start_shell" | "execute_shell" | "stop_shell" | "execute_command" => {
            shell::render_shell(out, result)
        }

        // Todos
        "write_todos" => render_todos(out, result),

        // Code execution
        "distri_execute_code" => code::render_code_execution(out, result),

        // Artifact tool
        "artifact_tool" => render_artifact(out, result),

        // HTTP request tool
        "http_request" => render_request(out, result),

        // Default: generic part-by-part rendering
        _ => render_tool_result(out, result),
    }
}

fn render_todos(out: &mut dyn Write, result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        if let Part::Data(value) = part
            && let Some(todos) = value.get("todos").and_then(|v| v.as_array())
        {
            outln!(
                out,
                "{}{}Updated Plan{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                COLOR_RESET
            );
            for todo in todos {
                let content = todo
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(missing)");
                let status = todo
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");
                let icon = match status {
                    "completed" | "done" => "■",
                    "in_progress" => "◐",
                    _ => "□",
                };
                outln!(out, "{}  {} {}{}", COLOR_GRAY, icon, content, COLOR_RESET);
            }
            return;
        }
    }
    render_tool_result(out, result);
}

fn render_request(out: &mut dyn Write, result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        if let Part::Data(value) = part {
            let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
            let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let body = value.get("body").unwrap_or(value);

            let preview = serde_json::to_string(body).unwrap_or_default();
            let max_len = if ok { 120 } else { 200 };
            let preview = if preview.len() > max_len {
                format!("{}...", truncate_str(&preview, max_len))
            } else {
                preview
            };

            if ok {
                outln!(
                    out,
                    "{}{}{} — {}{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    status,
                    preview,
                    COLOR_RESET
                );
            } else {
                outln!(
                    out,
                    "{}{}{} — {}{}",
                    COLOR_RED,
                    RESULT_PREFIX,
                    status,
                    preview,
                    COLOR_RESET
                );
            }
            return;
        }
    }
    render_tool_result(out, result);
}

fn render_artifact(out: &mut dyn Write, result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        match part {
            Part::Artifact(meta) => {
                outln!(
                    out,
                    "{}{}artifact: {} ({}){}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    meta.original_filename
                        .as_deref()
                        .unwrap_or(&meta.relative_path),
                    meta.content_type.as_deref().unwrap_or("unknown"),
                    COLOR_RESET,
                );
            }
            Part::Text(text) => {
                let preview: String = text.lines().take(1).collect();
                outln!(
                    out,
                    "{}{}{}{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    preview,
                    COLOR_RESET
                );
            }
            _ => {}
        }
    }
}
