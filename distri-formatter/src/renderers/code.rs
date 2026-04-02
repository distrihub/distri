use super::RESULT_PREFIX;
use crate::colors::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use distri_types::{Part, ToolResponse};

/// Render distri_execute_code results.
pub fn render_code_execution(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Text(text) => {
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
                if let Some(obj) = value.as_object() {
                    if let Some(code) = obj.get("exit_code").and_then(|c| c.as_i64()) {
                        let color = if code == 0 { COLOR_GREEN } else { COLOR_RED };
                        println!("{}{}exit: {}{}", color, RESULT_PREFIX, code, COLOR_RESET);
                    }
                    if let Some(stderr) = obj.get("stderr").and_then(|s| s.as_str())
                        && !stderr.trim().is_empty()
                    {
                        let first = stderr.lines().next().unwrap_or("");
                        println!(
                            "{}{}stderr: {}{}",
                            COLOR_RED, RESULT_PREFIX, first, COLOR_RESET
                        );
                    }
                }
            }
            Part::Artifact(meta) => {
                println!(
                    "{}{}output: {} ({}){}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    meta.original_filename
                        .as_deref()
                        .unwrap_or(&meta.relative_path),
                    meta.content_type.as_deref().unwrap_or("unknown"),
                    COLOR_RESET,
                );
            }
            _ => {}
        }
    }
}
