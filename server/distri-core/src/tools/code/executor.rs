use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::tools::shell::{
    BrowsrShellClient, CreateShellSessionRequest, ShellExecRequest,
};
use serde_json::Value;

#[derive(Clone)]
pub struct CodeExecutor {
    pub _context: Arc<ExecutorContext>,
}

impl CodeExecutor {
    pub fn new(context: Arc<ExecutorContext>) -> Self {
        Self { _context: context }
    }
}

/// Execute code using a browsr shell session.
///
/// Creates an ephemeral shell session, runs the code, captures stdout/stderr,
/// and destroys the session. Returns (result_value, observations, has_external_tools).
pub async fn execute_code_with_tools(
    code: &str,
    _context: Arc<ExecutorContext>,
) -> Result<(Value, Vec<String>, bool), anyhow::Error> {
    let client = BrowsrShellClient::from_env();

    // Detect language from code content (default to javascript for backward compat)
    let language = detect_language(code);

    // Create an ephemeral shell session
    let session = client
        .create_session(&CreateShellSessionRequest {
            language: Some(language.to_string()),
            image: None,
            memory_mb: None,
            disk_mb: None,
            cpu_cores: None,
            timeout_secs: Some(30),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create shell session: {}", e))?;

    let session_id = session.session_id.clone();

    // Wrap code for execution based on language
    let command = wrap_code_for_execution(code, language);

    // Execute the code
    let result = client
        .exec(&ShellExecRequest {
            session_id: session_id.clone(),
            command,
            timeout_secs: Some(20),
            working_dir: None,
        })
        .await;

    // Always clean up the session
    let _ = client.destroy_session(&session_id).await;

    let response = result.map_err(|e| anyhow::anyhow!("Shell execution failed: {}", e))?;

    // Collect observations from stdout
    let mut observations = Vec::new();
    if !response.stdout.is_empty() {
        observations.push(response.stdout.clone());
    }
    if !response.stderr.is_empty() {
        observations.push(format!("[stderr] {}", response.stderr));
    }

    // Build result value
    let result_value = serde_json::json!({
        "stdout": response.stdout,
        "stderr": response.stderr,
        "exit_code": response.exit_code,
        "duration_ms": response.duration_ms,
    });

    if response.exit_code != 0 {
        tracing::warn!(
            "Code execution exited with code {}: {}",
            response.exit_code,
            response.stderr
        );
    }

    Ok((result_value, observations, false))
}

/// Detect language from code content.
fn detect_language(code: &str) -> &'static str {
    let trimmed = code.trim();

    // Python indicators
    if trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("class ")
        || trimmed.contains("print(")
    {
        return "python";
    }

    // Bash indicators
    if trimmed.starts_with("#!/bin/")
        || trimmed.starts_with("apt ")
        || trimmed.starts_with("sudo ")
        || trimmed.starts_with("curl ")
        || trimmed.starts_with("wget ")
        || trimmed.contains("| grep")
    {
        return "bash";
    }

    // Default to javascript (backward compat with old JS sandbox)
    "javascript"
}

/// Wrap code for shell execution based on language.
fn wrap_code_for_execution(code: &str, language: &str) -> String {
    match language {
        "python" => format!("python3 -c {}", shell_escape(code)),
        "bash" => format!("bash -c {}", shell_escape(code)),
        "javascript" => format!("node -e {}", shell_escape(code)),
        _ => format!("bash -c {}", shell_escape(code)),
    }
}

fn shell_escape(s: &str) -> String {
    // Use single-quote wrapping with internal single-quote escaping
    format!("'{}'", s.replace('\'', "'\\''"))
}
