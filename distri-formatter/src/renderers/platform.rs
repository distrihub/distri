use super::RESULT_PREFIX;
use crate::colors::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use distri_types::{Part, ToolResponse};
use std::io::Write;

pub fn render_platform_tool(out: &mut dyn Write, result: &ToolResponse) {
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
                outln!(
                    out,
                    "{}{}Loaded \"{}\" ({} steps){}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    skill_name,
                    steps,
                    COLOR_RESET
                );
            } else {
                render_one_liner(out, text, false);
            }
        }
        "list_skills" => {
            let count = data
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            outln!(
                out,
                "{}{}Found {} skills{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                count,
                COLOR_RESET
            );
        }
        "list_agents" => {
            let count = data
                .and_then(|d| d.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            outln!(
                out,
                "{}{}Found {} agents{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                count,
                COLOR_RESET
            );
        }
        "create_skill" => {
            let skill_name = data
                .and_then(|d| d.get("name").and_then(|v| v.as_str()))
                .unwrap_or("skill");
            outln!(
                out,
                "{}{}Created \"{}\"{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                skill_name,
                COLOR_RESET
            );
        }
        "delete_skill" => {
            outln!(out, "{}{}Deleted{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
        "write_to_storage" => {
            outln!(out, "{}{}Saved{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
        "read_from_storage" => {
            if let Some(t) = text {
                let lines = t.lines().count();
                outln!(
                    out,
                    "{}{}({} lines){}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    lines,
                    COLOR_RESET
                );
            } else {
                outln!(out, "{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
            }
        }
        "tool_search" => {
            // `tool_search` returns its payload as `Part::Text(json_string)`
            // (the LLM consumes the JSON directly). Parse it to surface the
            // count rather than always printing 0.
            let count = text
                .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
                .and_then(|v| v.get("tools_found").and_then(|n| n.as_u64()))
                .unwrap_or(0);
            outln!(
                out,
                "{}{}Found {} tools{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                count,
                COLOR_RESET
            );
        }
        "inject_connection_env" => {
            if false {
                render_one_liner(out, text, true);
            } else {
                outln!(
                    out,
                    "{}{}Connected{}",
                    COLOR_GREEN,
                    RESULT_PREFIX,
                    COLOR_RESET
                );
            }
        }
        _ => render_one_liner(out, text, false),
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
    parts
        .iter()
        .find_map(|p| if let Part::Data(v) = p { Some(v) } else { None })
}

fn render_one_liner(out: &mut dyn Write, text: Option<&str>, is_error: bool) {
    let color = if is_error { COLOR_RED } else { COLOR_GRAY };
    match text {
        Some(t) => {
            let first_line = t.lines().next().unwrap_or("");
            let truncated = if first_line.len() > 100 {
                format!("{}...", super::truncate_str(first_line, 100))
            } else {
                first_line.to_string()
            };
            outln!(
                out,
                "{}{}{}{}",
                color,
                RESULT_PREFIX,
                truncated,
                COLOR_RESET
            );
        }
        None => {
            outln!(out, "{}{}Done{}", COLOR_GRAY, RESULT_PREFIX, COLOR_RESET);
        }
    }
}
