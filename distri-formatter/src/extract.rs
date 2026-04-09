//! Typed extraction and formatting of `ToolResponse` parts.
//!
//! `extract_fields` dispatches on `tool_name` and returns a `ToolFields` variant
//! that holds strongly-typed fields pulled from the `Part::Data(json)` payload.
//! `ToolFields::format_plain` produces clean, LLM-friendly scratchpad text.

use distri_types::{Part, ToolResponse};

// ---------------------------------------------------------------------------
// ToolFields enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ToolFields {
    Bash {
        stdout: String,
        stderr: String,
        exit_code: i64,
    },
    Read {
        content: String,
        file_path: String,
        lines_read: u64,
        total: u64,
        truncated: bool,
    },
    Grep {
        output: String,
        total_lines: u64,
        truncated: bool,
    },
    Glob {
        filenames: Vec<String>,
        num_files: u64,
        truncated: bool,
    },
    Edit {
        file_path: String,
        replacements: u64,
    },
    Write {
        file_path: String,
        bytes_written: u64,
    },
    Shell {
        stdout: String,
        stderr: String,
        exit_code: i64,
    },
    Generic {
        text: String,
    },
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extract typed fields from a `ToolResponse`.
pub fn extract_fields(response: &ToolResponse) -> ToolFields {
    match response.tool_name.as_str() {
        "Bash" => extract_bash(response),
        "Read" => extract_read(response),
        "Grep" => extract_grep(response),
        "Glob" => extract_glob(response),
        "Edit" => extract_edit(response),
        "Write" => extract_write(response),
        "execute_shell" | "execute_command" | "start_shell" | "stop_shell" => {
            extract_shell(response)
        }
        _ => extract_generic(response),
    }
}

fn first_data(response: &ToolResponse) -> Option<&serde_json::Value> {
    for part in &response.parts {
        if let Part::Data(v) = part {
            return Some(v);
        }
    }
    None
}

fn extract_bash(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Bash {
            stdout: str_field(v, "stdout"),
            stderr: str_field(v, "stderr"),
            exit_code: v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(-1),
        }
    } else {
        ToolFields::Bash {
            stdout: generic_text(response),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}

fn extract_read(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Read {
            content: str_field(v, "content"),
            file_path: str_field(v, "file_path"),
            lines_read: v.get("lines_read").and_then(|x| x.as_u64()).unwrap_or(0),
            total: v.get("total_lines").and_then(|x| x.as_u64()).unwrap_or(0),
            truncated: v
                .get("truncated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        }
    } else {
        ToolFields::Read {
            content: generic_text(response),
            file_path: String::new(),
            lines_read: 0,
            total: 0,
            truncated: false,
        }
    }
}

fn extract_grep(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Grep {
            output: str_field(v, "output"),
            total_lines: v.get("total_lines").and_then(|x| x.as_u64()).unwrap_or(0),
            truncated: v
                .get("truncated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        }
    } else {
        ToolFields::Grep {
            output: generic_text(response),
            total_lines: 0,
            truncated: false,
        }
    }
}

fn extract_glob(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        let filenames = v
            .get("filenames")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        ToolFields::Glob {
            filenames,
            num_files: v.get("num_files").and_then(|x| x.as_u64()).unwrap_or(0),
            truncated: v
                .get("truncated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        }
    } else {
        ToolFields::Glob {
            filenames: Vec::new(),
            num_files: 0,
            truncated: false,
        }
    }
}

fn extract_edit(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Edit {
            file_path: str_field(v, "file_path"),
            replacements: v.get("replacements").and_then(|x| x.as_u64()).unwrap_or(1),
        }
    } else {
        ToolFields::Edit {
            file_path: String::new(),
            replacements: 1,
        }
    }
}

fn extract_write(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Write {
            file_path: str_field(v, "file_path"),
            bytes_written: v.get("bytes_written").and_then(|x| x.as_u64()).unwrap_or(0),
        }
    } else {
        ToolFields::Write {
            file_path: String::new(),
            bytes_written: 0,
        }
    }
}

fn extract_shell(response: &ToolResponse) -> ToolFields {
    if let Some(v) = first_data(response) {
        ToolFields::Shell {
            stdout: str_field(v, "stdout"),
            stderr: str_field(v, "stderr"),
            exit_code: v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(-1),
        }
    } else {
        ToolFields::Shell {
            stdout: generic_text(response),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}

fn extract_generic(response: &ToolResponse) -> ToolFields {
    ToolFields::Generic {
        text: generic_text(response),
    }
}

/// Walk parts looking for text-like content. Checks `Part::Text` first, then
/// `Part::Data` for common text-bearing keys.
fn generic_text(response: &ToolResponse) -> String {
    // 1. Prefer Part::Text
    for part in &response.parts {
        if let Part::Text(t) = part {
            if !t.is_empty() {
                return t.clone();
            }
        }
    }
    // 2. Walk Part::Data for known text-bearing keys
    const TEXT_KEYS: &[&str] = &["content", "text", "output", "stdout", "message", "value"];
    for part in &response.parts {
        if let Part::Data(v) = part {
            for key in TEXT_KEYS {
                if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                    if !s.is_empty() {
                        return s.to_owned();
                    }
                }
            }
            // Fall back to the whole JSON serialised
            if let Ok(s) = serde_json::to_string(v) {
                if s != "null" && s != "{}" {
                    return s;
                }
            }
        }
    }
    String::new()
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_owned()
}

// ---------------------------------------------------------------------------
// ToolFields methods
// ---------------------------------------------------------------------------

impl ToolFields {
    /// Extract the main content string for persistence (used when content exceeds threshold).
    pub fn large_content(&self) -> String {
        match self {
            ToolFields::Bash { stdout, stderr, .. } => {
                if stderr.is_empty() {
                    stdout.clone()
                } else {
                    format!("{}\n--- stderr ---\n{}", stdout, stderr)
                }
            }
            ToolFields::Read { content, .. } => content.clone(),
            ToolFields::Grep { output, .. } => output.clone(),
            ToolFields::Shell { stdout, stderr, .. } => {
                if stderr.is_empty() {
                    stdout.clone()
                } else {
                    format!("{}\n--- stderr ---\n{}", stdout, stderr)
                }
            }
            ToolFields::Generic { text } => text.clone(),
            _ => String::new(), // Edit/Write/Glob don't have large content
        }
    }

    /// Total byte size of the primary content fields.
    /// Used for threshold checks (decide whether to persist to disk).
    pub fn content_size(&self) -> usize {
        match self {
            ToolFields::Bash { stdout, stderr, .. } => stdout.len() + stderr.len(),
            ToolFields::Shell { stdout, stderr, .. } => stdout.len() + stderr.len(),
            ToolFields::Read { content, .. } => content.len(),
            ToolFields::Grep { output, .. } => output.len(),
            ToolFields::Glob { filenames, .. } => filenames.iter().map(|f| f.len() + 1).sum(),
            ToolFields::Edit { .. } | ToolFields::Write { .. } => 0,
            ToolFields::Generic { text } => text.len(),
        }
    }

    /// Produce clean plain text for an LLM scratchpad.
    ///
    /// `max_chars` — hard cap on output length (0 = unlimited).
    /// `file_ref`  — if `Some((path, size_kb))`, a hint line is appended so the
    ///               LLM knows it can read the full output from disk.
    pub fn format_plain(&self, max_chars: usize, file_ref: Option<(&str, usize)>) -> String {
        let body = match self {
            ToolFields::Bash {
                stdout,
                stderr,
                exit_code,
            } => {
                format!(
                    "[Bash] exit_code={exit_code}\n<stdout>\n{stdout}\n</stdout>\n<stderr>\n{stderr}\n</stderr>"
                )
            }
            ToolFields::Read {
                content,
                file_path,
                lines_read,
                total,
                truncated,
            } => {
                let trunc_note = if *truncated {
                    format!(" (truncated, {total} total)")
                } else {
                    String::new()
                };
                format!("[Read: {file_path}] {lines_read} lines{trunc_note}\n{content}")
            }
            ToolFields::Grep {
                output,
                total_lines,
                truncated,
            } => {
                let suffix = if *truncated { " (truncated)" } else { "" };
                format!("[Grep] {total_lines} matches{suffix}\n{output}")
            }
            ToolFields::Glob {
                filenames,
                num_files,
                truncated,
            } => {
                let suffix = if *truncated { " (truncated)" } else { "" };
                let list = filenames
                    .iter()
                    .map(|f| format!("  {f}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("[Glob] {num_files} files{suffix}\n{list}")
            }
            ToolFields::Edit {
                file_path,
                replacements,
            } => {
                format!("[Edit: {file_path}] {replacements} replacement(s)")
            }
            ToolFields::Write {
                file_path,
                bytes_written,
            } => {
                format!("[Write: {file_path}] {bytes_written} bytes")
            }
            ToolFields::Shell {
                stdout,
                stderr,
                exit_code,
            } => {
                format!("[Shell] exit_code={exit_code}\n{stdout}\nstderr: {stderr}")
            }
            ToolFields::Generic { text } => {
                if text.is_empty() {
                    "(completed with no output)".to_owned()
                } else {
                    text.clone()
                }
            }
        };

        // Apply truncation budget
        let body = if max_chars > 0 && body.len() > max_chars {
            truncate_to_budget(&body, max_chars)
        } else {
            body
        };

        // Append file-ref hint
        if let Some((path, size_kb)) = file_ref {
            format!("{body}\n[Full output saved — Read(\"{path}\") for {size_kb}KB]")
        } else {
            body
        }
    }
}

/// Truncate `s` to at most `max` chars, preferring a newline boundary when
/// the last newline is past the 50% mark of the budget.
fn truncate_to_budget(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let total = s.len();
    // Target: leave room for the suffix
    let suffix = format!("...[truncated, {total} total chars]");
    let budget = max.saturating_sub(suffix.len());

    // Try to find a newline within [50%..budget]
    let half = budget / 2;
    let candidate = &s[..budget];
    let cut = candidate
        .rmatch_indices('\n')
        .find(|(pos, _)| *pos >= half)
        .map(|(pos, _)| pos)
        .unwrap_or(budget);

    format!("{}{}", &s[..cut], suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::ToolResponse;
    use serde_json::json;

    fn make_data_response(tool_name: &str, data: serde_json::Value) -> ToolResponse {
        ToolResponse::direct("call-1".to_owned(), tool_name.to_owned(), data)
    }

    fn make_text_response(tool_name: &str, text: &str) -> ToolResponse {
        ToolResponse::from_parts(
            "call-1".to_owned(),
            tool_name.to_owned(),
            vec![Part::Text(text.to_owned())],
        )
    }

    // ------------------------------------------------------------------
    // extract_fields — known tools
    // ------------------------------------------------------------------

    #[test]
    fn test_extract_bash() {
        let r = make_data_response(
            "Bash",
            json!({"stdout": "hello\n", "stderr": "warn", "exit_code": 0}),
        );
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Bash { ref stdout, ref stderr, exit_code: 0 }
            if stdout == "hello\n" && stderr == "warn"
        ));
    }

    #[test]
    fn test_extract_read() {
        let r = make_data_response(
            "Read",
            json!({"content": "line1\nline2", "file_path": "/foo.rs", "lines_read": 2, "total_lines": 100, "truncated": true}),
        );
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Read { ref content, ref file_path, lines_read: 2, total: 100, truncated: true }
            if content == "line1\nline2" && file_path == "/foo.rs"
        ));
    }

    #[test]
    fn test_extract_grep() {
        let r = make_data_response(
            "Grep",
            json!({"output": "match1\nmatch2", "total_lines": 2, "truncated": false}),
        );
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Grep { ref output, total_lines: 2, truncated: false }
            if output == "match1\nmatch2"
        ));
    }

    #[test]
    fn test_extract_glob() {
        let r = make_data_response(
            "Glob",
            json!({"filenames": ["a.rs", "b.rs"], "num_files": 2, "truncated": false}),
        );
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Glob { ref filenames, num_files: 2, truncated: false }
            if filenames == &["a.rs", "b.rs"]
        ));
    }

    #[test]
    fn test_extract_edit() {
        let r = make_data_response("Edit", json!({"file_path": "/bar.rs", "replacements": 3}));
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Edit { ref file_path, replacements: 3 }
            if file_path == "/bar.rs"
        ));
    }

    #[test]
    fn test_extract_write() {
        let r = make_data_response(
            "Write",
            json!({"file_path": "/out.txt", "bytes_written": 512}),
        );
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Write { ref file_path, bytes_written: 512 }
            if file_path == "/out.txt"
        ));
    }

    #[test]
    fn test_extract_shell() {
        for name in &[
            "execute_shell",
            "execute_command",
            "start_shell",
            "stop_shell",
        ] {
            let r = make_data_response(name, json!({"stdout": "ok", "stderr": "", "exit_code": 0}));
            let f = extract_fields(&r);
            assert!(
                matches!(f, ToolFields::Shell { .. }),
                "expected Shell for {name}"
            );
        }
    }

    // ------------------------------------------------------------------
    // extract_fields — unknown / generic
    // ------------------------------------------------------------------

    #[test]
    fn test_extract_unknown_data_tool() {
        let r = make_data_response("SomeFancyTool", json!({"output": "fancy result"}));
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Generic { ref text }
            if text == "fancy result"
        ));
    }

    #[test]
    fn test_extract_part_text() {
        let r = make_text_response("UnknownTool", "plain text result");
        let f = extract_fields(&r);
        assert!(matches!(
            f,
            ToolFields::Generic { ref text }
            if text == "plain text result"
        ));
    }

    // ------------------------------------------------------------------
    // format_plain
    // ------------------------------------------------------------------

    #[test]
    fn test_format_plain_bash_small_no_file_ref() {
        let fields = ToolFields::Bash {
            stdout: "hello".to_owned(),
            stderr: String::new(),
            exit_code: 0,
        };
        let out = fields.format_plain(0, None);
        assert!(out.contains("[Bash] exit_code=0"));
        assert!(out.contains("<stdout>\nhello\n</stdout>"));
        assert!(out.contains("<stderr>"));
        assert!(!out.contains("Full output saved"));
    }

    #[test]
    fn test_format_plain_bash_large_with_file_ref() {
        let big_stdout = "x".repeat(5000);
        let fields = ToolFields::Bash {
            stdout: big_stdout,
            stderr: String::new(),
            exit_code: 0,
        };
        // Allow 200 chars; file_ref provided
        let out = fields.format_plain(200, Some(("/tmp/out.txt", 5)));
        assert!(out.contains("Full output saved"));
        assert!(out.contains("Read(\"/tmp/out.txt\")"));
        assert!(out.contains("5KB"));
        // Must be under max_chars + file_ref appendage
        // (file_ref line is added after truncation so total can exceed max_chars slightly)
    }

    #[test]
    fn test_format_plain_bash_with_truncation() {
        let big = "line\n".repeat(1000); // 5000 chars
        let fields = ToolFields::Bash {
            stdout: big,
            stderr: String::new(),
            exit_code: 1,
        };
        let out = fields.format_plain(300, None);
        assert!(out.contains("truncated"));
        assert!(out.len() <= 300);
    }

    #[test]
    fn test_format_plain_generic_empty() {
        let fields = ToolFields::Generic {
            text: String::new(),
        };
        let out = fields.format_plain(0, None);
        assert_eq!(out, "(completed with no output)");
    }

    #[test]
    fn test_format_plain_read() {
        let fields = ToolFields::Read {
            content: "foo\nbar".to_owned(),
            file_path: "/src/main.rs".to_owned(),
            lines_read: 2,
            total: 200,
            truncated: true,
        };
        let out = fields.format_plain(0, None);
        assert!(out.contains("[Read: /src/main.rs]"));
        assert!(out.contains("2 lines"));
        assert!(out.contains("truncated"));
        assert!(out.contains("foo\nbar"));
    }

    #[test]
    fn test_format_plain_glob() {
        let fields = ToolFields::Glob {
            filenames: vec!["a.rs".to_owned(), "b.rs".to_owned()],
            num_files: 2,
            truncated: false,
        };
        let out = fields.format_plain(0, None);
        assert!(out.contains("[Glob] 2 files"));
        assert!(out.contains("  a.rs"));
        assert!(out.contains("  b.rs"));
    }

    #[test]
    fn test_format_plain_edit() {
        let fields = ToolFields::Edit {
            file_path: "/path/to/file.rs".to_owned(),
            replacements: 2,
        };
        let out = fields.format_plain(0, None);
        assert_eq!(out, "[Edit: /path/to/file.rs] 2 replacement(s)");
    }

    #[test]
    fn test_format_plain_write() {
        let fields = ToolFields::Write {
            file_path: "/out/file.txt".to_owned(),
            bytes_written: 1024,
        };
        let out = fields.format_plain(0, None);
        assert_eq!(out, "[Write: /out/file.txt] 1024 bytes");
    }

    // ------------------------------------------------------------------
    // content_size
    // ------------------------------------------------------------------

    #[test]
    fn test_content_size_bash() {
        let f = ToolFields::Bash {
            stdout: "abc".to_owned(),
            stderr: "de".to_owned(),
            exit_code: 0,
        };
        assert_eq!(f.content_size(), 5);
    }

    #[test]
    fn test_content_size_edit_write_zero() {
        let e = ToolFields::Edit {
            file_path: "/f".to_owned(),
            replacements: 1,
        };
        assert_eq!(e.content_size(), 0);
        let w = ToolFields::Write {
            file_path: "/f".to_owned(),
            bytes_written: 9999,
        };
        assert_eq!(w.content_size(), 0);
    }

    #[test]
    fn test_content_size_generic() {
        let f = ToolFields::Generic {
            text: "hello world".to_owned(),
        };
        assert_eq!(f.content_size(), 11);
    }
}
