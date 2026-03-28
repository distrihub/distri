use crate::printer::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use crate::renderers::RESULT_PREFIX;
use distri_types::{Part, ToolResponse};

pub fn render_platform_tool(result: &ToolResponse) {
    let name = result.tool_name.as_str();
    let text = first_text(&result.parts);
    let data = first_data(&result.parts);

    match name {
        "load_skill" => {
            if let Some(d) = data {
                let skill_name = d.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let steps = d
                    .get("steps")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!(
                    "{}{}Loaded \"{}\" ({} steps){}",
                    COLOR_GRAY, RESULT_PREFIX, skill_name, steps, COLOR_RESET
                );
            } else {
                render_one_liner(text, false);
            }
        }
        "run_skill_script" => {
            if false {
                render_one_liner(text, true);
            } else {
                println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
        "list_skills" => {
            let count = data
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "{}{}Found {} skills{}",
                COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET
            );
        }
        "list_agents" => {
            let count = data
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "{}{}Found {} agents{}",
                COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET
            );
        }
        "create_skill" => {
            let skill_name = data
                .and_then(|d| d.get("name").and_then(|v| v.as_str()))
                .unwrap_or("skill");
            println!(
                "{}{}Created \"{}\"{}",
                COLOR_GRAY, RESULT_PREFIX, skill_name, COLOR_RESET
            );
        }
        "delete_skill" => {
            println!("{}{}Deleted{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
        "write_to_storage" => {
            println!("{}{}Saved{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
        "read_from_storage" => {
            if let Some(t) = text {
                let lines = t.lines().count();
                println!(
                    "{}{}({} lines){}",
                    COLOR_GRAY, RESULT_PREFIX, lines, COLOR_RESET
                );
            } else {
                println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
        "tool_search" => {
            let count = data
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "{}{}Found {} tools{}",
                COLOR_GRAY, RESULT_PREFIX, count, COLOR_RESET
            );
        }
        "inject_connection_env" => {
            if false {
                render_one_liner(text, true);
            } else {
                println!("{}{}Connected{}", COLOR_GREEN, RESULT_PREFIX, COLOR_RESET);
            }
        }
        _ => render_one_liner(text, false),
    }
}

fn first_text(parts: &[Part]) -> Option<&str> {
    parts.iter().find_map(|p| {
        if let Part::Text(t) = p {
            Some(t.as_str())
        } else {
            None
        }
    })
}

fn first_data(parts: &[Part]) -> Option<&serde_json::Value> {
    parts.iter().find_map(|p| {
        if let Part::Data(v) = p {
            Some(v)
        } else {
            None
        }
    })
}

fn render_one_liner(text: Option<&str>, is_error: bool) {
    let color = if is_error { COLOR_RED } else { COLOR_GRAY };
    match text {
        Some(t) => {
            let first_line = t.lines().next().unwrap_or("");
            let truncated = if first_line.len() > 100 {
                format!("{}...", &first_line[..100])
            } else {
                first_line.to_string()
            };
            println!("{}{}{}{}", color, RESULT_PREFIX, truncated, COLOR_RESET);
        }
        None => {
            println!("{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
    }
}
