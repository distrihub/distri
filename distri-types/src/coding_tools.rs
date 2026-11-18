//! The coding tools' contract, shared by every host that runs them:
//! distri-cli's local tools (`distri-cli/src/local_tools`) and the server's
//! sandbox tools (`distri-core/src/tools/sandbox_tools.rs`).
//!
//! The names, JSON schemas, prompts and the pure parts of the work (how
//! `Read` numbers lines, how `Edit` replaces) live here so the model sees the
//! same tools wherever they run.

use serde::Deserialize;
use serde_json::{Value, json};

pub const BASH: &str = "Bash";
pub const READ: &str = "Read";
pub const WRITE: &str = "Write";
pub const EDIT: &str = "Edit";

/// Default and maximum `Bash` timeout, in milliseconds.
pub const BASH_DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const BASH_MAX_TIMEOUT_MS: u64 = 600_000;
/// Most lines one `Read` returns.
pub const READ_MAX_LINES: usize = 2000;

pub const BASH_PROMPT: &str = r#"Executes a bash command and returns its output.

IMPORTANT: Avoid using Bash to run `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands. Instead, use the dedicated tools:
 - File search: Use `Glob` (NOT find or ls)
 - Content search: Use `Grep` (NOT grep or rg)
 - Read files: Use `Read` (NOT cat/head/tail)
 - Edit files: Use `Edit` (NOT sed/awk)
 - Write files: Use `Write` (NOT echo or cat with heredoc)

Use Bash for: running scripts, installing packages, git operations, builds, tests, and other shell commands that have no dedicated tool equivalent.
- Always quote file paths that contain spaces.
- When issuing multiple independent commands, make multiple Bash calls in parallel. If commands depend on each other, chain with `&&`.
- You may specify an optional `timeout` in milliseconds (max 600000ms / 10 minutes). Default: 120000ms (2 minutes)."#;

pub const READ_PROMPT: &str = r#"Reads a file from the local filesystem.
- Results are returned using `cat -n` format, with line numbers starting at 1.
- By default reads up to 2000 lines from the beginning of the file.
- When you already know which part of the file you need, only read that part using `offset` and `limit`.
- The `file_path` can be absolute or relative to the workspace root."#;

pub const WRITE_PROMPT: &str = r#"Writes a file to the local filesystem. This will overwrite the existing file if there is one.
- If this is an existing file, you MUST use `Read` first to read the file's contents.
- Prefer the `Edit` tool for modifying existing files — it only changes the specific part. Only use `Write` to create new files or for complete rewrites.
- Creates parent directories automatically.
- NEVER create documentation files (*.md) or README files unless explicitly requested."#;

pub const EDIT_PROMPT: &str = r#"Performs exact string replacements in files.
- You must use `Read` at least once before editing a file. Read the file first so you know the exact content to match.
- When editing text from `Read` output, preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: line number + tab. Everything after that is the actual file content to match. Never include any part of the line number prefix in `old_string` or `new_string`.
- ALWAYS prefer editing existing files. NEVER write new files unless explicitly required.
- The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all: true` to change every instance.
- Use `replace_all` for replacing and renaming strings across the file (e.g. renaming a variable)."#;

pub fn bash_description() -> &'static str {
    "Execute a bash command and return its output."
}
pub fn read_description() -> &'static str {
    "Read a file from the local filesystem."
}
pub fn write_description() -> &'static str {
    "Write a file to the local filesystem."
}
pub fn edit_description() -> &'static str {
    "Perform exact string replacements in files."
}

pub fn bash_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": "The command to execute" },
            "timeout": { "type": "number", "description": "Optional timeout in milliseconds (max 600000)" }
        },
        "required": ["command"]
    })
}

pub fn read_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": { "type": "string", "description": "The path to the file to read" },
            "offset": { "type": "number", "description": "The line number to start reading from (0-based)" },
            "limit": { "type": "number", "description": "The number of lines to read" }
        },
        "required": ["file_path"]
    })
}

pub fn write_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": { "type": "string", "description": "The path to the file to write" },
            "content": { "type": "string", "description": "The content to write to the file" }
        },
        "required": ["file_path", "content"]
    })
}

pub fn edit_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_path": { "type": "string", "description": "The path to the file to edit" },
            "old_string": { "type": "string", "description": "The text to replace" },
            "new_string": { "type": "string", "description": "The replacement text" },
            "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)", "default": false }
        },
        "required": ["file_path", "old_string", "new_string"]
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct BashParams {
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
}

impl BashParams {
    pub fn timeout_ms(&self) -> u64 {
        self.timeout
            .unwrap_or(BASH_DEFAULT_TIMEOUT_MS)
            .min(BASH_MAX_TIMEOUT_MS)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadParams {
    pub file_path: String,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WriteParams {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

/// `Bash`'s result: `{stdout, stderr, exit_code}`.
pub fn bash_result(stdout: &str, stderr: &str, exit_code: i64) -> Value {
    json!({ "stdout": stdout, "stderr": stderr, "exit_code": exit_code })
}

/// `Read`'s result for a text file: `cat -n` numbered lines from `offset`
/// (0-based), at most `limit` (capped at [`READ_MAX_LINES`]).
pub fn read_text_result(
    file_path: &str,
    text: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Value {
    let all_lines: Vec<&str> = text.lines().collect();
    let total_lines = all_lines.len();
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(READ_MAX_LINES).min(READ_MAX_LINES);
    let start = offset.min(total_lines);
    let end = (start + limit).min(total_lines);
    let numbered: String = all_lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4}\t{}", start + i + 1, line))
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "content": numbered,
        "file_path": file_path,
        "total_lines": total_lines,
        "lines_read": end - start,
        "truncated": end < total_lines,
    })
}

/// `Write`'s result.
pub fn write_result(file_path: &str, bytes_written: usize) -> Value {
    json!({ "file_path": file_path, "bytes_written": bytes_written, "success": true })
}

/// Apply an `Edit` to `content`: returns the new content and the number of
/// replacements, or the error text the model sees.
pub fn apply_edit(content: &str, params: &EditParams) -> Result<(String, usize), String> {
    let count = content.matches(&params.old_string).count();
    if count == 0 {
        return Err(format!(
            "old_string not found in {}. Make sure it matches exactly.",
            params.file_path
        ));
    }
    if count > 1 && !params.replace_all {
        return Err(format!(
            "old_string found {} times in {}. Use replace_all: true to replace all, or provide more context to make it unique.",
            count, params.file_path
        ));
    }
    if params.replace_all {
        Ok((
            content.replace(&params.old_string, &params.new_string),
            count,
        ))
    } else {
        Ok((
            content.replacen(&params.old_string, &params.new_string, 1),
            1,
        ))
    }
}

/// `Edit`'s result.
pub fn edit_result(file_path: &str, replacements: usize) -> Value {
    json!({ "file_path": file_path, "replacements": replacements, "success": true })
}

/// The MIME type for an image path, by extension.
pub fn image_mime_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_numbers_lines() {
        let v = read_text_result("f", "a\nb\nc\n", Some(1), Some(1));
        assert_eq!(v["content"], "   2\tb");
        assert_eq!(v["total_lines"], 3);
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn edit_rules() {
        let p = |old: &str, all| EditParams {
            file_path: "f".into(),
            old_string: old.into(),
            new_string: "X".into(),
            replace_all: all,
        };
        assert!(apply_edit("aa", &p("b", false)).is_err());
        assert!(apply_edit("aa", &p("a", false)).is_err());
        assert_eq!(apply_edit("aa", &p("a", true)).unwrap(), ("XX".into(), 2));
        assert_eq!(apply_edit("ab", &p("a", false)).unwrap(), ("Xb".into(), 1));
    }
}
