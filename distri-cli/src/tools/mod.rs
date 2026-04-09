mod bash;
mod edit;
mod glob;
mod grep;
pub mod prompts;
mod read;
mod write;

use std::collections::HashMap;
use std::io::{self, Write as IoWrite};
use std::path::Path;

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolDefinition, ToolResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW, COLOR_GRAY, COLOR_RESET,
};

/// Names of all tools the CLI registers locally.
/// Used to ensure the stream client intercepts these tool calls
/// regardless of which agent is running.
pub const LOCAL_TOOL_NAMES: &[&str] = &[
    "Bash", "Read", "Write", "Edit", "Glob", "Grep", "execute_command",
];

/// Register all local CLI tools and return their definitions (with prompts).
pub fn register_all(
    registry: &ExternalToolRegistry,
    _agent_id: &str,
    workspace_root: &Path,
) -> Vec<ToolDefinition> {
    // Register under "*" so handlers are available to ALL agents,
    // not just the initially-launched one.
    bash::register(registry, "*", workspace_root);
    read::register(registry, "*", workspace_root);
    write::register(registry, "*", workspace_root);
    edit::register(registry, "*", workspace_root);
    glob::register(registry, "*", workspace_root);
    grep::register(registry, "*", workspace_root);
    register_execute_command(registry, "*", workspace_root);

    tool_definitions()
}

/// Build ToolDefinitions for all local CLI tools (with prompt instructions).
fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "Bash".into(),
            description: "Execute a bash command and return its output.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout": { "type": "number", "description": "Optional timeout in milliseconds (max 600000)" }
                },
                "required": ["command"]
            }),
            prompt: Some(prompts::BASH_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
        ToolDefinition {
            name: "Read".into(),
            description: "Read a file from the local filesystem.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The path to the file to read" },
                    "offset": { "type": "number", "description": "The line number to start reading from (0-based)" },
                    "limit": { "type": "number", "description": "The number of lines to read" }
                },
                "required": ["file_path"]
            }),
            prompt: Some(prompts::READ_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
        ToolDefinition {
            name: "Write".into(),
            description: "Write a file to the local filesystem.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The path to the file to write" },
                    "content": { "type": "string", "description": "The content to write to the file" }
                },
                "required": ["file_path", "content"]
            }),
            prompt: Some(prompts::WRITE_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
        ToolDefinition {
            name: "Edit".into(),
            description: "Perform exact string replacements in files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "The path to the file to edit" },
                    "old_string": { "type": "string", "description": "The text to replace" },
                    "new_string": { "type": "string", "description": "The replacement text" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)", "default": false }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            prompt: Some(prompts::EDIT_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
        ToolDefinition {
            name: "Glob".into(),
            description: "Fast file pattern matching tool.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The glob pattern to match files against" },
                    "path": { "type": "string", "description": "The directory to search in (defaults to workspace root)" }
                },
                "required": ["pattern"]
            }),
            prompt: Some(prompts::GLOB_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
        ToolDefinition {
            name: "Grep".into(),
            description: "Search file contents with regex (ripgrep).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The regex pattern to search for" },
                    "path": { "type": "string", "description": "File or directory to search in" },
                    "glob": { "type": "string", "description": "Glob pattern to filter files" },
                    "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Output mode (default: files_with_matches)" },
                    "-A": { "type": "number", "description": "Lines after match" },
                    "-B": { "type": "number", "description": "Lines before match" },
                    "-C": { "type": "number", "description": "Context lines" },
                    "-i": { "type": "boolean", "description": "Case insensitive" },
                    "-n": { "type": "boolean", "description": "Show line numbers" },
                    "type": { "type": "string", "description": "File type filter (js, py, rust, etc.)" },
                    "head_limit": { "type": "number", "description": "Limit output lines (default 250)" },
                    "multiline": { "type": "boolean", "description": "Enable multiline matching" }
                },
                "required": ["pattern"]
            }),
            prompt: Some(prompts::GREP_PROMPT.into()),
            examples: None,
            output_schema: None,
        },
    ]
}

pub fn register_approval_handler(registry: &ExternalToolRegistry) {
    registry.register("*", "approval_request", |call, _event| async move {
        println!(
            "{}Calling tool:{} {}",
            COLOR_BRIGHT_MAGENTA, COLOR_RESET, call.tool_name
        );
        println!("{}Approval required{}", COLOR_BRIGHT_YELLOW, COLOR_RESET);
        print!(
            "{}Do you approve this operation? (y/n): {}",
            COLOR_BRIGHT_YELLOW, COLOR_RESET
        );
        io::stdout().flush().ok();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return Err(anyhow::anyhow!("Failed to read approval input"));
        }

        let approved = input.trim().eq_ignore_ascii_case("y");
        if approved {
            println!(
                "{}Operation approved by user.{}",
                COLOR_BRIGHT_GREEN, COLOR_RESET
            );
        } else {
            println!("Operation rejected by user.");
        }

        let tool_calls = call.input.clone();
        let approval_result = json!({
            "approved": approved,
            "reason": if approved { "Approved by user" } else { "Rejected by user" },
            "tool_calls": tool_calls,
        });

        Ok(ToolResponse::direct(
            call.tool_call_id.clone(),
            call.tool_name.clone(),
            approval_result,
        ))
    });
}

// ---------------------------------------------------------------------------
// ExecuteCommandTool — local shell execution (legacy name for backward compat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExecuteCommandParams {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

/// Register the `execute_command` tool for local shell execution in a workspace.
pub fn register_execute_command(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    workspace_root: &Path,
) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "execute_command".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: ExecuteCommandParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid execute_command parameters: {}", e))?;

                let mut working_dir = workspace.clone();
                if let Some(ref relative) = params.cwd {
                    let trimmed = relative.trim();
                    if !trimmed.is_empty() && trimmed != "." {
                        working_dir = working_dir.join(trimmed);
                    }
                }
                std::fs::create_dir_all(&working_dir).with_context(|| {
                    format!("failed to create working directory {:?}", working_dir)
                })?;

                let mut command = if cfg!(target_os = "windows") {
                    let mut cmd = tokio::process::Command::new("cmd");
                    cmd.arg("/C").arg(&params.command);
                    cmd
                } else {
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg("-lc").arg(&params.command);
                    cmd
                };

                command.current_dir(&working_dir);
                if let Some(env_map) = &params.env {
                    for (key, value) in env_map {
                        command.env(key, value);
                    }
                }

                let output = command
                    .output()
                    .await
                    .with_context(|| format!("failed to execute command '{}'", params.command))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or_default();

                let response = json!({
                    "command": params.command,
                    "cwd": params.cwd.unwrap_or_else(|| ".".to_string()),
                    "exit_code": exit_code,
                    "success": output.status.success(),
                    "stdout": stdout,
                    "stderr": stderr
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "execute_command".to_string(),
                    vec![Part::Data(response)],
                ))
            }
        },
    );
}

/// Validate that all external tools declared in the agent definition are registered locally.
/// Prints available/missing tools and returns an error if any are missing.
pub fn validate_external_tools(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    required: &std::collections::HashSet<String>,
    verbose: bool,
) -> anyhow::Result<()> {
    if required.is_empty() {
        return Ok(());
    }

    let mut available = Vec::new();
    let mut missing = Vec::new();

    for name in required {
        if registry.has_tool(agent_id, name) {
            available.push(name.as_str());
        } else {
            missing.push(name.as_str());
        }
    }

    available.sort();
    missing.sort();

    if verbose {
        println!(
            "{}External tools ({} registered, {} missing){}",
            COLOR_GRAY,
            available.len(),
            missing.len(),
            COLOR_RESET
        );
        for name in &available {
            println!("  {}✓{} {}", COLOR_BRIGHT_GREEN, COLOR_RESET, name);
        }
        for name in &missing {
            println!("  {}✗{} {}", COLOR_BRIGHT_YELLOW, COLOR_RESET, name);
        }
    }

    if !missing.is_empty() {
        anyhow::bail!(
            "Agent '{}' requires external tools not available locally: {}",
            agent_id,
            missing.join(", ")
        );
    }

    Ok(())
}
