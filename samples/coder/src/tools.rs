use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use distri_types::{Part, Tool, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct ExecuteCommandTool {
    code_home: PathBuf,
}

impl ExecuteCommandTool {
    pub fn new<P: AsRef<Path>>(code_home: P) -> Self {
        Self {
            code_home: code_home.as_ref().to_path_buf(),
        }
    }

    fn resolve_working_dir(&self, cwd: Option<String>) -> Result<PathBuf> {
        let mut dir = self.code_home.clone();
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecuteCommandParams {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

#[async_trait]
impl Tool for ExecuteCommandTool {
    fn get_name(&self) -> String {
        "execute_command".to_string()
    }

    fn get_description(&self) -> String {
        "Execute a shell command inside the CODE_HOME workspace".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory relative to CODE_HOME",
                    "default": "."
                },
                "env": {
                    "type": "object",
                    "description": "Optional environment variables to set for the command",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let params: ExecuteCommandParams = serde_json::from_value(tool_call.input.clone())
            .map_err(|e| anyhow!("invalid execute_command parameters: {}", e))?;

        let working_dir = self.resolve_working_dir(params.cwd.clone())?;

        let mut command = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&params.command);
            cmd
        } else {
            let mut cmd = Command::new("bash");
            cmd.arg("-lc").arg(&params.command);
            cmd
        };

        command.current_dir(working_dir);

        if let Some(env_map) = params.env {
            for (key, value) in env_map {
                command.env(key, value);
            }
        }

        let output = command
            .output()
            .await
            .with_context(|| format!("failed to execute command '{}", params.command))?;

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

        Ok(vec![Part::Data(response)])
    }
}
