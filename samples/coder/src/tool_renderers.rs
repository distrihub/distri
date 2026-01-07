use std::sync::Arc;

use distri_cli::tool_renderers::{ToolCallStartContext, ToolRendererFn};

const COLOR_BLUE: &str = "\x1b[94m";
const COLOR_CYAN: &str = "\x1b[96m";
const COLOR_RESET: &str = "\x1b[0m";

pub fn coder_renderers() -> Vec<(String, ToolRendererFn)> {
    vec![
        (
            "execute_command".to_string(),
            Arc::new(|context: ToolCallStartContext<'_>| render_execute_command(context)),
        ),
        (
            "write_todos".to_string(),
            Arc::new(|context: ToolCallStartContext<'_>| render_write_todos(context)),
        ),
    ]
}

fn render_execute_command(context: ToolCallStartContext<'_>) -> Option<String> {
    let command = context.input.get("command")?.as_str()?.trim();
    let cwd = context
        .input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or(".");
    Some(format!(
        "{COLOR_BLUE}⏺ exec `{}` (cwd: {}){COLOR_RESET}",
        command, cwd
    ))
}

fn render_write_todos(context: ToolCallStartContext<'_>) -> Option<String> {
    let mut todo_lines = Vec::new();

    if let Some(entries) = context
        .input
        .get("todos")
        .and_then(|value| value.as_array())
    {
        for todo in entries {
            let content = todo
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("(missing todo)");
            let status = todo
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
            todo_lines.push(format!("{} {}", todo_status_icon(status), content));
        }
    }

    Some(format_plan_block("write_todos", &todo_lines))
}

fn format_plan_block(action: &str, lines: &[String]) -> String {
    let heading = todo_action_label(action);
    let mut rendered = format!("{COLOR_CYAN}• {}{COLOR_RESET}\n", heading);

    if lines.is_empty() {
        rendered.push_str("  └ (No todos tracked)\n");
    } else {
        for (index, line) in lines.iter().enumerate() {
            let prefix = if index == 0 { "  └" } else { "    " };
            rendered.push_str(&format!("{} {}\n", prefix, line));
        }
    }

    rendered
}

fn todo_status_icon(status: &str) -> &'static str {
    match status {
        "completed" | "done" => "■",
        "in_progress" => "◐",
        _ => "□",
    }
}

fn todo_action_label(action: &str) -> &'static str {
    match action {
        "add" => "New Plan",
        "update" | "write" | "write_todos" => "Updated Plan",
        "remove" => "Plan Updated",
        "clear" => "Plan Cleared",
        "list" => "Plan Snapshot",
        _ => "Plan Update",
    }
}
