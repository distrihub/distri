use super::RESULT_PREFIX;
use crate::colors::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use crate::extract::{ToolFields, extract_fields};
use distri_types::{Part, ToolResponse};

/// Render shell tool results (start_shell, execute_shell, stop_shell).
pub fn render_shell(result: &ToolResponse) {
    let name = result.tool_name.as_str();

    match name {
        "start_shell" => {
            println!(
                "{}{}Shell started{}",
                COLOR_GREEN, RESULT_PREFIX, COLOR_RESET
            );
        }
        "stop_shell" => {
            println!(
                "{}{}Shell stopped{}",
                COLOR_GRAY, RESULT_PREFIX, COLOR_RESET
            );
        }
        "execute_shell" | "execute_command" => {
            let has_data = result.parts.iter().any(|p| matches!(p, Part::Data(_)));
            let fields = extract_fields(result);
            if let ToolFields::Shell {
                stdout,
                stderr,
                exit_code,
            } = fields
            {
                if !stdout.trim().is_empty() {
                    let lines: Vec<&str> = stdout.lines().collect();
                    let total = lines.len();
                    for line in lines.iter().take(5) {
                        println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                    }
                    if total > 5 {
                        println!("{}  … ({} lines total){}", COLOR_GRAY, total, COLOR_RESET);
                    }
                }

                if has_data {
                    if !stderr.trim().is_empty() {
                        let first = stderr.lines().next().unwrap_or("");
                        println!(
                            "{}{}stderr: {}{}",
                            COLOR_RED, RESULT_PREFIX, first, COLOR_RESET
                        );
                    }

                    let color = if exit_code == 0 {
                        COLOR_GREEN
                    } else {
                        COLOR_RED
                    };
                    println!(
                        "{}{}exit: {}{}",
                        color, RESULT_PREFIX, exit_code, COLOR_RESET
                    );
                }
            }
        }
        _ => {
            crate::renderers::tool_result::render_tool_result(result);
        }
    }
}
