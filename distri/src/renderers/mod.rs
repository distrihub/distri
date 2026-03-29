mod browser;
mod code;
mod data;
mod platform;
mod search;
mod shell;
mod tool_result;

pub use tool_result::render_tool_result;

use crate::printer::{COLOR_GRAY, COLOR_RESET};
use distri_types::ToolResponse;

/// Result line prefix: `  ⎿  ` (matches Claude Code style)
pub const RESULT_PREFIX: &str = "  ⎿  ";

/// Dispatch tool result rendering to the appropriate tool-specific renderer.
pub fn render_tool_output(result: &ToolResponse, verbose: bool) {
    if verbose {
        if let Ok(json) = serde_json::to_string_pretty(&result.parts) {
            println!("{}Tool result{}:\n{}", COLOR_GRAY, COLOR_RESET, json);
        }
        return;
    }

    let name = result.tool_name.as_str();

    match name {
        // Simple tools — suppress output (streamed text is enough)
        "final" | "reflect" | "console_log" | "transfer_to_agent" => {}

        // Platform / discovery tools
        "tool_search" | "load_skill" | "run_skill_script" | "list_agents" | "list_skills"
        | "create_skill" | "delete_skill" | "write_to_storage" | "read_from_storage"
        | "inject_connection_env" => {
            platform::render_platform_tool(result);
        }

        // Browser / scraping
        "browsr_scrape" | "browsr_crawl" => browser::render_scrape(result),
        "browsr_browser" | "browser_step" => browser::render_browser_step(result),

        // Search
        "search" => search::render_search(result),

        // Shell
        "start_shell" | "execute_shell" | "stop_shell" => shell::render_shell(result),

        // Code execution
        "distri_execute_code" => code::render_code_execution(result),

        // Artifact tool
        "artifact_tool" => render_artifact(result),

        // HTTP request tools
        "request" => render_request(result),

        // Default: generic part-by-part rendering
        _ => render_tool_result(result),
    }
}

fn render_request(result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        if let Part::Data(value) = part {
            let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
            let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

            if ok {
                // Show status and compact preview of data
                let data = value.get("data").unwrap_or(value);
                let preview = serde_json::to_string(data).unwrap_or_default();
                let preview = if preview.len() > 120 {
                    format!("{}…", &preview[..120])
                } else {
                    preview
                };
                println!(
                    "{}{}✓ {} — {}{}",
                    COLOR_GRAY, RESULT_PREFIX, status, preview, COLOR_RESET
                );
            } else {
                // Show error with the actual content
                let error = value.get("error").unwrap_or(value);
                let preview = serde_json::to_string(error).unwrap_or_default();
                let preview = if preview.len() > 200 {
                    format!("{}…", &preview[..200])
                } else {
                    preview
                };
                println!(
                    "{}{}✗ {} — {}{}",
                    crate::printer::COLOR_RED, RESULT_PREFIX, status, preview, COLOR_RESET
                );
            }
            return;
        }
    }
    // Fallback
    render_tool_result(result);
}

fn render_artifact(result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        match part {
            Part::Artifact(meta) => {
                println!(
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
                println!("{}{}{}{}", COLOR_GRAY, RESULT_PREFIX, preview, COLOR_RESET);
            }
            _ => {}
        }
    }
}
