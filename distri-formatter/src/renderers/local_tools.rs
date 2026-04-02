//! Renderers for local CLI tools (Bash, Read, Write, Edit, Glob, Grep).
//! Follows claude-code style: compact, informative, consistent prefix.

use super::{RESULT_PREFIX, truncate_str};
use crate::colors::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use distri_types::{Part, ToolResponse};

pub fn render_local_tool(result: &ToolResponse) {
    match result.tool_name.as_str() {
        "Bash" => render_bash(result),
        "Read" => render_read(result),
        "Write" => render_write(result),
        "Edit" => render_edit(result),
        "Glob" => render_glob(result),
        "Grep" => render_grep(result),
        _ => super::tool_result::render_tool_result(result),
    }
}

fn render_bash(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let exit_code = value
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            let stdout = value.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let stderr = value.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

            // Show stdout (first 5 lines)
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

            // Show stderr
            if !stderr.trim().is_empty() {
                let first = stderr.lines().next().unwrap_or("");
                println!(
                    "{}{}stderr: {}{}",
                    COLOR_RED,
                    RESULT_PREFIX,
                    truncate_str(first, 120),
                    COLOR_RESET
                );
            }

            // Show exit code if non-zero
            if exit_code != 0 {
                println!(
                    "{}{}exit: {}{}",
                    COLOR_RED, RESULT_PREFIX, exit_code, COLOR_RESET
                );
            }
            return;
        }
    }
}

fn render_read(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let lines_read = value
                .get("lines_read")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let total = value
                .get("total_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let truncated = value
                .get("truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if truncated {
                println!(
                    "{}{}Read {} lines (of {} total){}",
                    COLOR_GRAY, RESULT_PREFIX, lines_read, total, COLOR_RESET
                );
            } else {
                println!(
                    "{}{}Read {} lines{}",
                    COLOR_GRAY, RESULT_PREFIX, lines_read, COLOR_RESET
                );
            }
            return;
        }
    }
}

fn render_write(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let path = value
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let bytes = value
                .get("bytes_written")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!(
                "{}{}Wrote {} bytes to {}{}",
                COLOR_GREEN, RESULT_PREFIX, bytes, path, COLOR_RESET
            );
            return;
        }
    }
}

fn render_edit(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let path = value
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let replacements = value
                .get("replacements")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            if replacements == 1 {
                println!(
                    "{}{}Updated {}{}",
                    COLOR_GREEN, RESULT_PREFIX, path, COLOR_RESET
                );
            } else {
                println!(
                    "{}{}Updated {} ({} replacements){}",
                    COLOR_GREEN, RESULT_PREFIX, path, replacements, COLOR_RESET
                );
            }
            return;
        }
    }
}

fn render_glob(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let num_files = value.get("num_files").and_then(|v| v.as_u64()).unwrap_or(0);
            let truncated = value
                .get("truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let filenames = value.get("filenames").and_then(|v| v.as_array());

            let suffix = if truncated { " (truncated)" } else { "" };
            println!(
                "{}{}{} files{}{}",
                COLOR_GRAY, RESULT_PREFIX, num_files, suffix, COLOR_RESET
            );

            // Show first few filenames
            if let Some(files) = filenames {
                for f in files.iter().take(5) {
                    if let Some(name) = f.as_str() {
                        println!("{}  {}{}", COLOR_GRAY, name, COLOR_RESET);
                    }
                }
                if files.len() > 5 {
                    println!(
                        "{}  … and {} more{}",
                        COLOR_GRAY,
                        files.len() - 5,
                        COLOR_RESET
                    );
                }
            }
            return;
        }
    }
}

fn render_grep(result: &ToolResponse) {
    for part in &result.parts {
        if let Part::Data(value) = part {
            let total_lines = value
                .get("total_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let truncated = value
                .get("truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let output = value.get("output").and_then(|v| v.as_str()).unwrap_or("");

            let suffix = if truncated { " (truncated)" } else { "" };
            println!(
                "{}{}{} results{}{}",
                COLOR_GRAY, RESULT_PREFIX, total_lines, suffix, COLOR_RESET
            );

            // Show first few lines of output
            if !output.is_empty() {
                let lines: Vec<&str> = output.lines().collect();
                for line in lines.iter().take(8) {
                    println!("{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                }
                if lines.len() > 8 {
                    println!(
                        "{}  … and {} more lines{}",
                        COLOR_GRAY,
                        lines.len() - 8,
                        COLOR_RESET
                    );
                }
            }
            return;
        }
    }
}
