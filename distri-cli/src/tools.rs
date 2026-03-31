use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolDefinition, ToolResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW, COLOR_RESET};

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
// ExecuteCommandTool — local shell execution (CLI-specific)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExecuteCommandParams {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

fn resolve_working_dir(workspace_root: &Path, cwd: Option<&str>) -> anyhow::Result<PathBuf> {
    let mut dir = workspace_root.to_path_buf();
    if let Some(relative) = cwd {
        let trimmed = relative.trim();
        if !trimmed.is_empty() {
            dir = dir.join(trimmed);
        }
    }
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create working directory at {:?}", dir))?;
    Ok(dir)
}

/// Register the `execute_command` tool for local shell execution in a workspace.
pub fn register_execute_command_tool(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    workspace_root: &Path,
) -> ToolDefinition {
    let workspace = workspace_root.to_path_buf();

    let definition = ToolDefinition {
        name: "execute_command".to_string(),
        description: "Execute a shell command in the workspace directory".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory relative to workspace root",
                    "default": "."
                },
                "env": {
                    "type": "object",
                    "description": "Optional environment variables to set for the command",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["command"]
        }),
        output_schema: None,
        examples: None,
    };

    let def_clone = definition.clone();
    registry.register(
        agent_id.to_string(),
        "execute_command".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: ExecuteCommandParams =
                    serde_json::from_value(call.input.clone())
                        .map_err(|e| anyhow::anyhow!("invalid execute_command parameters: {}", e))?;

                let working_dir = resolve_working_dir(&workspace, params.cwd.as_deref())?;

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

    def_clone
}
