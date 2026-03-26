use crate::printer::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use distri_types::{Part, ToolResponse};

/// Render shell tool results (start_shell, execute_shell, stop_shell).
pub fn render_shell(result: &ToolResponse) {
    let name = result.tool_name.as_str();

    match name {
        "start_shell" => {
            for part in &result.parts {
                if let Part::Text(text) = part {
                    println!(
                        "{}  shell started: {}{}",
                        COLOR_GREEN,
                        text.trim(),
                        COLOR_RESET
                    );
                }
            }
        }
        "stop_shell" => {
            println!("{}  shell stopped{}", COLOR_GRAY, COLOR_RESET);
        }
        "execute_shell" => {
            for part in &result.parts {
                match part {
                    Part::Text(text) => {
                        // Show output preview (first few lines)
                        let lines: Vec<&str> = text.lines().collect();
                        let total = lines.len();
                        for line in lines.iter().take(5) {
                            println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                        }
                        if total > 5 {
                            println!("{}  … ({} lines total){}", COLOR_GRAY, total, COLOR_RESET);
                        }
                    }
                    Part::Data(value) => {
                        // Check for exit code
                        if let Some(obj) = value.as_object() {
                            if let Some(code) = obj.get("exit_code").and_then(|c| c.as_i64()) {
                                let color = if code == 0 { COLOR_GREEN } else { COLOR_RED };
                                println!("{}  exit: {}{}", color, code, COLOR_RESET);
                            }
                            if let Some(stderr) = obj.get("stderr").and_then(|s| s.as_str()) {
                                if !stderr.trim().is_empty() {
                                    let lines: Vec<&str> = stderr.lines().take(3).collect();
                                    for line in &lines {
                                        println!("{}  stderr: {}{}", COLOR_RED, line, COLOR_RESET);
                                    }
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
