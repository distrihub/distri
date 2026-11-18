//! Per-tool prompt instructions (injected into the system prompt via {{tool_prompts}}).
//! Mirrors claude-code's per-tool prompt() pattern.

pub use distri_types::coding_tools::{BASH_PROMPT, EDIT_PROMPT, READ_PROMPT, WRITE_PROMPT};

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
