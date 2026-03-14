use crate::printer::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use distri_types::{Part, ToolResponse};

/// Render distri_execute_code results — shows output and exit status.
pub fn render_code_execution(result: &ToolResponse) {
    for part in &result.parts {
        match part {
            Part::Text(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let total = lines.len();
                for line in lines.iter().take(8) {
                    println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                }
                if total > 8 {
                    println!("{}  … ({} lines total){}", COLOR_GRAY, total, COLOR_RESET);
                }
            }
            Part::Data(value) => {
                if let Some(obj) = value.as_object() {
                    if let Some(code) = obj.get("exit_code").and_then(|c| c.as_i64()) {
                        let color = if code == 0 { COLOR_GREEN } else { COLOR_RED };
                        println!("{}  exit: {}{}", color, code, COLOR_RESET);
                    }
                    if let Some(stdout) = obj.get("stdout").and_then(|s| s.as_str()) {
                        if !stdout.trim().is_empty() {
                            let lines: Vec<&str> = stdout.lines().take(5).collect();
                            for line in &lines {
                                println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                            }
                            if stdout.lines().count() > 5 {
                                println!("{}  …{}", COLOR_GRAY, COLOR_RESET);
                            }
                        }
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
            Part::Artifact(meta) => {
                println!(
                    "{}  output file: {} ({}){}", COLOR_GRAY,
                    meta.original_filename.as_deref().unwrap_or(&meta.relative_path),
                    meta.content_type.as_deref().unwrap_or("unknown"),
                    COLOR_RESET,
                );
            }
            _ => {}
        }
    }
}
