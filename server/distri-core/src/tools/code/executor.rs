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

    // Send raw code — browsr-shell wraps with the appropriate interpreter
    let command = code.to_string();

    // Execute the code
    let result = client
        .exec(&ShellExecRequest {
            session_id: session_id.clone(),
            command,
            timeout_secs: Some(20),
            working_dir: None,
        })
        .await;

    // Destroy session in background — don't block the tool response
    let destroy_client = client.clone();
    let destroy_id = session_id.clone();
    tokio::spawn(async move {
        let _ = destroy_client.destroy_session(&destroy_id).await;
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Language detection ───────────────────────────────────────

    #[test]
    fn detect_python_import() {
        assert_eq!(detect_language("import math\nprint(math.pi)"), "python");
    }

    #[test]
    fn detect_python_from_import() {
        assert_eq!(detect_language("from os import path"), "python");
    }

    #[test]
    fn detect_python_def() {
        assert_eq!(detect_language("def foo():\n  return 42"), "python");
    }

    #[test]
    fn detect_python_class() {
        assert_eq!(detect_language("class Foo:\n  pass"), "python");
    }

    #[test]
    fn detect_python_print() {
        assert_eq!(detect_language("x = 1\nprint(x)"), "python");
    }

    #[test]
    fn detect_bash_shebang() {
        assert_eq!(detect_language("#!/bin/bash\necho hello"), "bash");
    }

    #[test]
    fn detect_bash_apt() {
        assert_eq!(detect_language("apt install -y curl"), "bash");
    }

    #[test]
    fn detect_bash_sudo() {
        assert_eq!(detect_language("sudo rm -rf /tmp/test"), "bash");
    }

    #[test]
    fn detect_bash_curl() {
        assert_eq!(detect_language("curl https://example.com"), "bash");
    }

    #[test]
    fn detect_bash_pipe_grep() {
        assert_eq!(detect_language("cat file.txt | grep pattern"), "bash");
    }

    #[test]
    fn detect_javascript_default() {
        assert_eq!(detect_language("console.log('hello')"), "javascript");
    }

    #[test]
    fn detect_javascript_const() {
        assert_eq!(detect_language("const x = 42;"), "javascript");
    }

    #[test]
    fn detect_language_trims_whitespace() {
        assert_eq!(detect_language("  \n  import os\n"), "python");
    }

}
