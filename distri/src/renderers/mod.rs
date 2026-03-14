mod browser;
mod code;
mod data;
mod search;
mod shell;
mod tool_result;

pub use tool_result::render_tool_result;

use crate::printer::{COLOR_GRAY, COLOR_RESET};
use distri_types::ToolResponse;

/// Dispatch tool result rendering to the appropriate tool-specific renderer.
/// Returns true if a specialized renderer handled it, false to fall back to generic.
pub fn render_tool_output(result: &ToolResponse, verbose: bool) {
    if verbose {
        if let Ok(json) = serde_json::to_string_pretty(&result.parts) {
            println!("{}Tool result{}:\n{}", COLOR_GRAY, COLOR_RESET, json);
        }
        return;
    }

    let name = result.tool_name.as_str();

    match name {
        // Browser / scraping tools
        "browsr_scrape" | "browsr_crawl" => browser::render_scrape(result),
        "browsr_browser" | "browser_step" => browser::render_browser_step(result),

        // Search
        "search" => search::render_search(result),

        // Shell
        "start_shell" | "execute_shell" | "stop_shell" => shell::render_shell(result),

        // Code execution
        "distri_execute_code" => code::render_code_execution(result),

        // Simple tools — just show text parts compactly
        "final" | "reflect" | "console_log" | "transfer_to_agent" => {
            render_simple_text(result);
        }

        // Artifact tool
        "artifact_tool" => render_artifact(result),

        // Discovery/platform tools
        "tool_search" | "load_skill" | "run_skill_script" | "list_agents" | "list_skills"
        | "create_skill" | "delete_skill" | "write_to_storage" | "read_from_storage" => {
            render_tool_result(result);
        }

        // Default: generic part-by-part rendering
        _ => render_tool_result(result),
    }
}

fn render_simple_text(result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        if let Part::Text(text) = part {
            let lines: Vec<&str> = text.lines().take(3).collect();
            let preview = lines.join("\n");
            if text.lines().count() > 3 {
                println!("{}  {}\n  …{}", COLOR_GRAY, preview, COLOR_RESET);
            } else {
                println!("{}  {}{}", COLOR_GRAY, preview, COLOR_RESET);
            }
        }
    }
}

fn render_artifact(result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        match part {
            Part::Artifact(meta) => {
                println!(
                    "{}  artifact: {} ({}){}",
                    COLOR_GRAY,
                    meta.original_filename
                        .as_deref()
                        .unwrap_or(&meta.relative_path),
                    meta.content_type.as_deref().unwrap_or("unknown"),
                    COLOR_RESET,
                );
            }
            Part::Text(text) => {
                let preview: String = text.lines().take(1).collect();
                println!("{}  {}{}", COLOR_GRAY, preview, COLOR_RESET);
            }
            _ => {}
        }
    }
}
