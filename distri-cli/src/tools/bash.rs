use std::path::Path;

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct BashParams {
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    description: Option<String>,
}

/// Register the `Bash` tool for local shell execution.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Bash".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: BashParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Bash parameters: {}", e))?;

                let timeout_ms = params.timeout.unwrap_or(120_000).min(600_000);

                let mut cmd = if cfg!(target_os = "windows") {
                    let mut c = tokio::process::Command::new("cmd");
                    c.arg("/C").arg(&params.command);
                    c
                } else {
                    let mut c = tokio::process::Command::new("bash");
                    c.arg("-lc").arg(&params.command);
                    c
                };

                cmd.current_dir(&workspace);

                let output = tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    cmd.output(),
                )
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "command timed out after {}ms: {}",
                        timeout_ms,
                        params.command
                    )
                })?
                .with_context(|| format!("failed to execute: {}", params.command))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                let response = json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": exit_code,
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Bash".to_string(),
                    vec![Part::Data(response)],
                ))
            }
        },
    );
}
