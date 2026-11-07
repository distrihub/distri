//! The CLI's built-in local tools (`Bash`, `Read`, `Write`, `Edit`, `Glob`,
//! `Grep`, `execute_command`) and the `approval_request` handler, registered
//! on an [`ExternalToolRegistry`] so the agent can call them over the network.
//!
//! Every call goes through the [`PermissionEngine`] first; a refused call
//! returns an error whose text the model reads as the tool result.

mod bash;
mod edit;
mod glob;
mod grep;
pub mod prompts;
mod read;
mod write;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolDefinition, ToolResponse};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::interact::{Interaction, Reply, SharedBroker};
use crate::permissions::PermissionEngine;

/// The engine and who it asks, shared by every handler.
#[derive(Clone)]
pub(crate) struct Gate {
    engine: PermissionEngine,
    broker: SharedBroker,
}

impl Gate {
    /// `Ok` when the call may run; otherwise the error the model reads.
    pub(crate) async fn authorize(&self, tool: &str, call: &ToolCall) -> anyhow::Result<()> {
        self.engine
            .authorize(&*self.broker, tool, &call.input)
            .await
            .map_err(|denied| anyhow::anyhow!(denied.message))
    }
}

/// Register all local CLI tools and return their definitions (with prompts).
pub fn register_all(
    registry: &ExternalToolRegistry,
    _agent_id: &str,
    workspace_root: &Path,
    engine: PermissionEngine,
    broker: SharedBroker,
) -> Vec<ToolDefinition> {
    let gate = Gate { engine, broker };
    // Register under "*" so handlers are available to ALL agents,
    // not just the initially-launched one.
    bash::register(registry, "*", workspace_root, gate.clone());
    read::register(registry, "*", workspace_root, gate.clone());
    write::register(registry, "*", workspace_root, gate.clone());
    edit::register(registry, "*", workspace_root, gate.clone());
    glob::register(registry, "*", workspace_root, gate.clone());
    grep::register(registry, "*", workspace_root, gate.clone());
    register_execute_command(registry, "*", workspace_root, gate);

    tool_definitions()
}

/// ToolDefinitions for the built-in local tools (with prompt instructions).
pub fn tool_definitions() -> Vec<ToolDefinition> {
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

// ---------------------------------------------------------------------------
// approval_request
// ---------------------------------------------------------------------------

/// The input of an `approval_request` call: the calls to approve and why.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// What an `approval_request` call returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalResult {
    pub approved: bool,
    pub reason: String,
    /// The calls the answer is about, as requested.
    pub tool_calls: Vec<ToolCall>,
}

impl ApprovalResult {
    fn from_reply(reply: Reply, tool_calls: Vec<ToolCall>) -> Self {
        let (approved, reason) = match reply {
            Reply::Allow { .. } => (true, "Approved by user".to_string()),
            Reply::Text(text) if is_yes(&text) => (true, "Approved by user".to_string()),
            Reply::Text(text) => (false, text),
            Reply::Deny { reason } => (
                false,
                reason
                    .filter(|r| !r.trim().is_empty())
                    .unwrap_or_else(|| "Rejected by user".to_string()),
            ),
            Reply::Unavailable => (
                false,
                "Nobody could be asked to approve (no terminal)".to_string(),
            ),
        };
        Self {
            approved,
            reason,
            tool_calls,
        }
    }
}

fn is_yes(text: &str) -> bool {
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "approve" | "approved" | "allow"
    )
}

/// Answer `approval_request` calls by asking `broker`.
pub fn register_approval_handler(registry: &ExternalToolRegistry, broker: SharedBroker) {
    registry.register("*", "approval_request", move |call: ToolCall, _event| {
        let broker = broker.clone();
        async move {
            let request = serde_json::from_value::<Option<ApprovalRequest>>(call.input.clone())
                .map_err(|e| anyhow::anyhow!("invalid approval_request parameters: {e}"))?
                .unwrap_or_default();
            let reply = broker
                .ask(Interaction::Approve {
                    calls: request.tool_calls.clone(),
                    reason: request.reason,
                })
                .await;
            let result = ApprovalResult::from_reply(reply, request.tool_calls);
            Ok(ToolResponse::direct(
                call.tool_call_id,
                call.tool_name,
                serde_json::to_value(result)?,
            ))
        }
    });
}

// ---------------------------------------------------------------------------
// execute_command — local shell execution (legacy name for backward compat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExecuteCommandParams {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct ExecuteCommandOutput {
    command: String,
    cwd: String,
    exit_code: i32,
    success: bool,
    stdout: String,
    stderr: String,
}

/// Register the `execute_command` tool for local shell execution in a workspace.
fn register_execute_command(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    workspace_root: &Path,
    gate: Gate,
) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "execute_command".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            let gate = gate.clone();
            async move {
                gate.authorize("execute_command", &call).await?;
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

                let response = ExecuteCommandOutput {
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    exit_code: output.status.code().unwrap_or_default(),
                    success: output.status.success(),
                    cwd: params.cwd.unwrap_or_else(|| ".".to_string()),
                    command: params.command,
                };

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "execute_command".to_string(),
                    vec![Part::Data(serde_json::to_value(response)?)],
                ))
            }
        },
    );
}
