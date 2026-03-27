use crate::printer::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use crate::renderers::RESULT_PREFIX;
use distri_types::{Part, ToolResponse};

/// Render shell tool results (start_shell, execute_shell, stop_shell).
pub fn render_shell(result: &ToolResponse) {
    let name = result.tool_name.as_str();

    match name {
        "start_shell" => {
            println!("{}{}Shell started{}", COLOR_GREEN, RESULT_PREFIX, COLOR_RESET);
        }
        "stop_shell" => {
            println!(
                "{}{}Shell stopped{}",
                COLOR_GRAY, RESULT_PREFIX, COLOR_RESET
            );
        }
        "execute_shell" => {
            for part in &result.parts {
                match part {
                    Part::Text(text) => {
                        let lines: Vec<&str> = text.lines().collect();
                        let total = lines.len();
                        for line in lines.iter().take(5) {
                            println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                        }
                        if total > 5 {
                            println!(
                                "{}  … ({} lines total){}",
                                COLOR_GRAY, total, COLOR_RESET
                            );
                        }
                    }
                    Part::Data(value) => {
                        if let Some(obj) = value.as_object() {
                            if let Some(code) = obj.get("exit_code").and_then(|c| c.as_i64()) {
                                let color = if code == 0 { COLOR_GREEN } else { COLOR_RED };
                                println!("{}{}exit: {}{}", color, RESULT_PREFIX, code, COLOR_RESET);
                            }
                            if let Some(stderr) = obj.get("stderr").and_then(|s| s.as_str()) {
                                if !stderr.trim().is_empty() {
                                    let first = stderr.lines().next().unwrap_or("");
                                    println!(
                                        "{}{}stderr: {}{}",
                                        COLOR_RED, RESULT_PREFIX, first, COLOR_RESET
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {
            crate::renderers::tool_result::render_tool_result(result);
        }
    }
}
