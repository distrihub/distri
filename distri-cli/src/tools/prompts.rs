//! Per-tool prompt instructions (injected into the system prompt via {{tool_prompts}}).
//! Mirrors claude-code's per-tool prompt() pattern.

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

pub const GLOB_PROMPT: &str = r#"Fast file pattern matching tool that works with any codebase size.
- Supports glob patterns like `**/*.js` or `src/**/*.ts`.
- Returns matching file paths sorted by modification time.
- Use this tool when you need to find files by name patterns.
- Use `Glob` instead of running `find` or `ls` in `Bash`."#;

pub const GREP_PROMPT: &str = r#"Powerful search tool built on ripgrep.
- ALWAYS use Grep for content search tasks. NEVER invoke `grep` or `rg` as a Bash command.
- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`).
- Filter files with `glob` parameter (e.g., `*.js`) or `type` parameter (e.g., `py`, `rust`).
- Output modes: `content` shows matching lines, `files_with_matches` shows only file paths (default), `count` shows match counts.
- Use `-A`, `-B`, `-C` for context lines around matches.
- For multiline patterns, use `multiline: true`."#;
