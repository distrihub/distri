use std::sync::Arc;

use distri_types::{Part, ToolCall, tool::ToolContext};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::tools::shell::{
    BrowsrShellClient, CreateShellSessionRequest, ShellExecRequest,
};
use crate::AgentError;

/// Tool that loads a skill's content on demand.
/// The agent calls this tool when it needs a specific skill.
/// The skill's markdown content is returned as-is, including
/// any instructions and tool usage details embedded within.
#[derive(Debug, Clone)]
pub struct LoadSkillTool;

#[async_trait::async_trait]
impl distri_types::Tool for LoadSkillTool {
    fn get_name(&self) -> String {
        "load_skill".to_string()
    }

    fn get_description(&self) -> String {
        "Load a skill by its ID. Returns the skill's full content including instructions, tool usage documentation, and available scripts.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The ID of the skill to load"
                }
            },
            "required": ["skill_id"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "LoadSkillTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for LoadSkillTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let skill_id = tool_call
            .input
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: skill_id".to_string())
            })?;

        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| {
                AgentError::ToolExecution(format!("Skill '{}' not found", skill_id))
            })?;

        // Build response: content + list of available scripts
        let mut response = skill.content.clone();
        if !skill.scripts.is_empty() {
            response.push_str("\n\n## Available Scripts\n");
            response.push_str("Use `run_skill_script` to execute these in a shell session:\n\n");
            for script in &skill.scripts {
                response.push_str(&format!(
                    "- **{}** ({}): {}\n",
                    script.name,
                    script.language,
                    script.description.as_deref().unwrap_or("No description"),
                ));
            }
            response.push_str("\nNote: Scripts are transferred to the remote shell and executed. ");
            response.push_str("A shell session will be started automatically if needed.");
        }

        Ok(vec![Part::Text(response)])
    }
}

/// Tool that runs a script from a skill in a remote shell session.
///
/// The script files are transferred to the remote shell container at
/// `/workspace/skills/<skill_name>/` and executed with the appropriate interpreter.
///
/// If no shell session is active, one is started automatically and destroyed after execution.
/// If a shell session is already active (via start_shell), it reuses that session.
#[derive(Debug, Clone)]
pub struct RunSkillScriptTool;

#[async_trait::async_trait]
impl distri_types::Tool for RunSkillScriptTool {
    fn get_name(&self) -> String {
        "run_skill_script".to_string()
    }

    fn get_description(&self) -> String {
        "Run a script from a skill in a remote shell session. The script is transferred to the shell container and executed with the appropriate interpreter. Provide input as JSON that the script can read from /tmp/skill_input.json.".to_string()
    }

    fn get_parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "The ID of the skill containing the script"
                },
                "script_name": {
                    "type": "string",
                    "description": "The name of the script to run"
                },
                "input": {
                    "type": "object",
                    "description": "Input parameters passed to the script as /tmp/skill_input.json"
                }
            },
            "required": ["skill_id", "script_name"]
        })
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "RunSkillScriptTool requires ExecutorContext, not ToolContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for RunSkillScriptTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let skill_id = tool_call
            .input
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: skill_id".to_string())
            })?;

        let script_name = tool_call
            .input
            .get("script_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing required parameter: script_name".to_string())
            })?;

        let input = tool_call
            .input
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Load the skill and find the script
        let orchestrator = context.get_orchestrator()?;
        let skill_store = orchestrator
            .stores
            .skill_store
            .as_ref()
            .ok_or_else(|| {
                AgentError::ToolExecution("Skill store not configured".to_string())
            })?;

        let skill = skill_store
            .get_skill(skill_id)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to load skill: {}", e)))?
            .ok_or_else(|| {
                AgentError::ToolExecution(format!("Skill '{}' not found", skill_id))
            })?;

        let script = skill
            .scripts
            .iter()
            .find(|s| s.name == script_name)
            .ok_or_else(|| {
                let available: Vec<_> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
                AgentError::ToolExecution(format!(
                    "Script '{}' not found in skill '{}'. Available scripts: {:?}",
                    script_name, skill.name, available
                ))
            })?;

        // Build env vars from context + process env
        let mut env_vars: std::collections::HashMap<String, String> = context
            .env_vars
            .clone()
            .unwrap_or_default();
        for key in &["BROWSR_BASE_URL", "BROWSR_API_URL", "BROWSR_API_KEY"] {
            if !env_vars.contains_key(*key) {
                if let Ok(val) = std::env::var(key) {
                    env_vars.insert(key.to_string(), val);
                }
            }
        }

        run_script_in_shell(
            &skill.name,
            &script.code,
            &script.language,
            &input,
            &script.name,
            &env_vars,
        )
        .await
    }
}

/// Execute a script by transferring it to a browsr shell session and running it.
///
/// Flow:
/// 1. Start a shell session
/// 2. Create /workspace/skills/<skill_name>/ directory
/// 3. Write the script file and input.json
/// 4. Set env vars
/// 5. Execute with the appropriate interpreter
/// 6. Return stdout/stderr
/// 7. Destroy the session
async fn run_script_in_shell(
    skill_name: &str,
    code: &str,
    language: &str,
    input: &serde_json::Value,
    script_name: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> Result<Vec<Part>, AgentError> {
    let client = BrowsrShellClient::from_env_with_vars(env_vars);

    // 1. Create a shell session
    let session = client
        .create_session(&CreateShellSessionRequest {
            image: None,
            memory_mb: None,
            disk_mb: None,
            cpu_cores: None,
            timeout_secs: Some(120),
            language: Some(language.to_string()),
        })
        .await
        .map_err(|e| {
            AgentError::ToolExecution(format!(
                "Failed to create shell session for script '{}': {}",
                script_name, e
            ))
        })?;

    let session_id = session.session_id.clone();

    // Run the script transfer + execution, always clean up session afterward
    let result = transfer_and_execute(
        &client,
        &session_id,
        skill_name,
        code,
        language,
        input,
        script_name,
        env_vars,
    )
    .await;

    // Always destroy the session
    let _ = client.destroy_session(&session_id).await;

    result
}

async fn transfer_and_execute(
    client: &BrowsrShellClient,
    session_id: &str,
    skill_name: &str,
    code: &str,
    language: &str,
    input: &serde_json::Value,
    script_name: &str,
    env_vars: &std::collections::HashMap<String, String>,
) -> Result<Vec<Part>, AgentError> {
    let skill_dir = format!(
        "/workspace/skills/{}",
        skill_name.replace(' ', "_").replace('/', "_")
    );

    // Determine file extension and interpreter
    let (ext, interpreter) = match language {
        "python" | "python3" => ("py", "python3"),
        "javascript" | "js" | "node" => ("js", "node"),
        "typescript" | "ts" => ("ts", "npx --yes tsx"),
        "bash" | "sh" => ("sh", "bash"),
        "ruby" | "rb" => ("rb", "ruby"),
        _ => ("sh", "bash"),
    };

    let script_filename = format!("{}.{}", script_name.replace(' ', "_"), ext);
    let script_path = format!("{}/{}", skill_dir, script_filename);
    let input_path = format!("{}/input.json", skill_dir);

    // Create skill directory
    exec_cmd(client, session_id, &format!("mkdir -p {}", skill_dir), 10).await?;

    // Write input.json
    let input_json = serde_json::to_string(input).unwrap_or_else(|_| "null".to_string());
    exec_cmd(
        client,
        session_id,
        &format!(
            "cat > {} << 'DISTRI_INPUT_EOF'\n{}\nDISTRI_INPUT_EOF",
            input_path, input_json,
        ),
        10,
    )
    .await?;

    // Set env vars
    let env_exports: Vec<String> = env_vars
        .iter()
        .map(|(k, v)| format!("export {}={}", k, shell_escape(v)))
        .collect();
    if !env_exports.is_empty() {
        exec_cmd(client, session_id, &env_exports.join("\n"), 10).await?;
    }

    // Transfer the script file
    exec_cmd(
        client,
        session_id,
        &format!(
            "cat > {} << 'DISTRI_SCRIPT_EOF'\n{}\nDISTRI_SCRIPT_EOF",
            script_path, code,
        ),
        10,
    )
    .await?;

    // Execute the script
    let run_cmd = format!("cd {} && {} {}", skill_dir, interpreter, script_filename);
    let response = client
        .exec(&ShellExecRequest {
            session_id: session_id.to_string(),
            command: run_cmd,
            timeout_secs: Some(60),
            working_dir: None,
        })
        .await
        .map_err(|e| {
            AgentError::ToolExecution(format!("Script '{}' execution failed: {}", script_name, e))
        })?;

    let result = json!({
        "stdout": response.stdout,
        "stderr": response.stderr,
        "exit_code": response.exit_code,
        "duration_ms": response.duration_ms,
        "timed_out": response.timed_out,
    });

    if response.exit_code != 0 {
        Ok(vec![Part::Text(format!(
            "Script '{}' failed (exit code {}):\nstdout: {}\nstderr: {}",
            script_name, response.exit_code, response.stdout, response.stderr
        ))])
    } else {
        Ok(vec![Part::Data(result)])
    }
}

/// Helper to execute a command in a session, ignoring output.
async fn exec_cmd(
    client: &BrowsrShellClient,
    session_id: &str,
    command: &str,
    timeout_secs: u32,
) -> Result<(), AgentError> {
    client
        .exec(&ShellExecRequest {
            session_id: session_id.to_string(),
            command: command.to_string(),
            timeout_secs: Some(timeout_secs),
            working_dir: None,
        })
        .await
        .map_err(|e| AgentError::ToolExecution(format!("Shell command failed: {}", e)))?;
    Ok(())
}

/// Simple shell escaping for env var values
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
