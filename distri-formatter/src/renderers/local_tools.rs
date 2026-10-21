//! Renderers for local CLI tools (Bash, Read, Write, Edit, Glob, Grep).
//! Follows claude-code style: compact, informative, consistent prefix.

use std::io::Write;

use super::{RESULT_PREFIX, truncate_str};
use crate::colors::{COLOR_GRAY, COLOR_GREEN, COLOR_RED, COLOR_RESET};
use crate::extract::{ToolFields, extract_fields};
use distri_types::ToolResponse;

pub fn render_local_tool(out: &mut dyn Write, result: &ToolResponse) {
    let fields = extract_fields(result);
    match fields {
        ToolFields::Bash {
            stdout,
            stderr,
            exit_code,
        } => {
            // Show stdout (first 5 lines)
            if !stdout.trim().is_empty() {
                let lines: Vec<&str> = stdout.lines().collect();
                let total = lines.len();
                for line in lines.iter().take(5) {
                    outln!(out, "{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                }
                if total > 5 {
                    outln!(
                        out,
                        "{}  … ({} lines total){}",
                        COLOR_GRAY,
                        total,
                        COLOR_RESET
                    );
                }
            }

            // Show stderr
            if !stderr.trim().is_empty() {
                let first = stderr.lines().next().unwrap_or("");
                outln!(
                    out,
                    "{}{}stderr: {}{}",
                    COLOR_RED,
                    RESULT_PREFIX,
                    truncate_str(first, 120),
                    COLOR_RESET
                );
            }

            // Show exit code if non-zero
            if exit_code != 0 {
                outln!(
                    out,
                    "{}{}exit: {}{}",
                    COLOR_RED,
                    RESULT_PREFIX,
                    exit_code,
                    COLOR_RESET
                );
            }
        }
        ToolFields::Read {
            lines_read,
            total,
            truncated,
            ..
        } => {
            if truncated {
                outln!(
                    out,
                    "{}{}Read {} lines (of {} total){}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    lines_read,
                    total,
                    COLOR_RESET
                );
            } else {
                outln!(
                    out,
                    "{}{}Read {} lines{}",
                    COLOR_GRAY,
                    RESULT_PREFIX,
                    lines_read,
                    COLOR_RESET
                );
            }
        }
        ToolFields::Write {
            file_path,
            bytes_written,
        } => {
            outln!(
                out,
                "{}{}Wrote {} bytes to {}{}",
                COLOR_GREEN,
                RESULT_PREFIX,
                bytes_written,
                file_path,
                COLOR_RESET
            );
        }
        ToolFields::Edit {
            file_path,
            replacements,
        } => {
            if replacements == 1 {
                outln!(
                    out,
                    "{}{}Updated {}{}",
                    COLOR_GREEN,
                    RESULT_PREFIX,
                    file_path,
                    COLOR_RESET
                );
            } else {
                outln!(
                    out,
                    "{}{}Updated {} ({} replacements){}",
                    COLOR_GREEN,
                    RESULT_PREFIX,
                    file_path,
                    replacements,
                    COLOR_RESET
                );
            }
        }
        ToolFields::Glob {
            filenames,
            num_files,
            truncated,
        } => {
            let suffix = if truncated { " (truncated)" } else { "" };
            outln!(
                out,
                "{}{}{} files{}{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                num_files,
                suffix,
                COLOR_RESET
            );

            // Show first few filenames
            for name in filenames.iter().take(5) {
                outln!(out, "{}  {}{}", COLOR_GRAY, name, COLOR_RESET);
            }
            if filenames.len() > 5 {
                outln!(
                    out,
                    "{}  … and {} more{}",
                    COLOR_GRAY,
                    filenames.len() - 5,
                    COLOR_RESET
                );
            }
        }
        ToolFields::Grep {
            output,
            total_lines,
            truncated,
        } => {
            let suffix = if truncated { " (truncated)" } else { "" };
            outln!(
                out,
                "{}{}{} results{}{}",
                COLOR_GRAY,
                RESULT_PREFIX,
                total_lines,
                suffix,
                COLOR_RESET
            );

            // Show first few lines of output
            if !output.is_empty() {
                let lines: Vec<&str> = output.lines().collect();
                for line in lines.iter().take(8) {
                    outln!(out, "{}  {}{}", COLOR_GRAY, line, COLOR_RESET);
                }
                if lines.len() > 8 {
                    outln!(
                        out,
                        "{}  … and {} more lines{}",
                        COLOR_GRAY,
                        lines.len() - 8,
                        COLOR_RESET
                    );
                }
            }
        }
        _ => super::tool_result::render_tool_result(out, result),
    }
}
