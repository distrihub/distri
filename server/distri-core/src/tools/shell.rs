use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use anyhow::Result;
use browsr_types::{
    ShellCreateSessionRequest, ShellCreateSessionResponse, ShellExecRequest, ShellExecResponse,
};
use distri_stores::SessionStoreExt;
use distri_types::{Part, Tool, ToolContext};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

const SHELL_SESSION_KEY: &str = "shell_session_id";

// ============================================================
// Browsr Shell HTTP Client
// ============================================================

#[derive(Clone)]
pub(crate) struct BrowsrShellClient {
    client: reqwest::Client,
    base_url: String,
}

impl BrowsrShellClient {
    pub(crate) fn from_env() -> Self {
        let base_url = std::env::var("BROWSR_BASE_URL")
            .or_else(|_| std::env::var("BROWSR_API_URL"))
            .unwrap_or_else(|_| "https://api.browsr.dev".to_string());

        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(api_key) = std::env::var("BROWSR_API_KEY") {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&api_key) {
                headers.insert("x-api-key", val);
            }
        }

        let has_key = headers.contains_key("x-api-key");
        tracing::info!(
            "[BrowsrShellClient::from_env] base_url={}, has_api_key={}",
            base_url,
            has_key
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("Failed to build HTTP client");

        Self { client, base_url }
    }

    pub(crate) async fn create_session(
        &self,
        request: &ShellCreateSessionRequest,
    ) -> Result<ShellCreateSessionResponse, AgentError> {
        let url = format!("{}/shell/sessions", self.base_url);
        tracing::info!("[BrowsrShellClient::create_session] POST {}", url);
        let resp = self
            .client
            .post(&url)
            .json(request)
            .send()
            .await
            .map_err(|e| {
                AgentError::ToolExecution(format!("Shell session creation failed: {}", e))
            })?;

        tracing::info!(
            "[BrowsrShellClient::create_session] response status={}",
            resp.status()
        );

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::error!("[BrowsrShellClient::create_session] failed: {}", text);
            return Err(AgentError::ToolExecution(format!(
                "Shell session creation failed: {}",
                text
            )));
        }

        resp.json().await.map_err(|e| {
            AgentError::ToolExecution(format!("Failed to parse session response: {}", e))
        })
    }

    pub(crate) async fn exec(
        &self,
        request: &ShellExecRequest,
    ) -> Result<ShellExecResponse, AgentError> {
        let url = format!("{}/shell/exec", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(request)
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Shell exec failed: {}", e)))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::ToolExecution(format!(
                "Shell exec failed: {}",
                text
            )));
        }

        resp.json()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("Failed to parse exec response: {}", e)))
    }

    pub(crate) async fn destroy_session(&self, session_id: &str) -> Result<(), AgentError> {
        let url = format!("{}/shell/sessions/{}", self.base_url, session_id);
        let resp = self.client.delete(&url).send().await.map_err(|e| {
            AgentError::ToolExecution(format!("Shell session deletion failed: {}", e))
        })?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::ToolExecution(format!(
                "Shell session deletion failed: {}",
                text
            )));
        }

        Ok(())
    }
}

// Shell API types are imported from browsr_types:
// ShellCreateSessionRequest, ShellCreateSessionResponse,
// ShellExecRequest, ShellExecResponse,
// NetworkAccess, ToolDefinition

// ============================================================
// Shell property overrides from context.tool_metadata["shell"]
// ============================================================

/// Shell overrides from tool_metadata["shell"]. Flat struct that merges
/// into `ShellCreateSessionRequest` — any field set here wins over defaults.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ShellOverrides {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub disk_mb: Option<u32>,
    #[serde(default)]
    pub cpu_cores: Option<f32>,
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub setup_script: Option<String>,
    #[serde(default)]
    pub cache_paths: Option<Vec<String>>,
    #[serde(default)]
    pub network_access: Option<browsr_types::NetworkAccess>,
    #[serde(default)]
    pub tools_endpoint: Option<String>,
    #[serde(default)]
    pub tool_timeout_secs: Option<u32>,
    #[serde(default)]
    pub tools: Option<Vec<browsr_types::ToolDefinition>>,
}

fn get_shell_overrides(context: &ExecutorContext) -> ShellOverrides {
    context
        .tool_metadata
        .as_ref()
        .and_then(|m| m.get("shell"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

// ============================================================
// Helper: get/set shell session from session store
// ============================================================

async fn get_shell_session_id(context: &ExecutorContext) -> Result<Option<String>, AgentError> {
    let session_store = context.get_session_store()?;
    let val: Option<String> = session_store
        .get(&context.thread_id, SHELL_SESSION_KEY)
        .await
        .map_err(|e| AgentError::ToolExecution(format!("Failed to read shell session: {}", e)))?;
    Ok(val.filter(|s| !s.is_empty()))
}

async fn set_shell_session_id(
    context: &ExecutorContext,
    session_id: &str,
) -> Result<(), AgentError> {
    let session_store = context.get_session_store()?;
    session_store
        .set(
            &context.thread_id,
            SHELL_SESSION_KEY,
            &session_id.to_string(),
        )
        .await
        .map_err(|e| AgentError::ToolExecution(format!("Failed to store shell session: {}", e)))
}

async fn clear_shell_session_id(context: &ExecutorContext) -> Result<(), AgentError> {
    let session_store = context.get_session_store()?;
    session_store
        .set(&context.thread_id, SHELL_SESSION_KEY, &String::new())
        .await
        .map_err(|e| AgentError::ToolExecution(format!("Failed to clear shell session: {}", e)))
}

// ============================================================
// StartShellTool
// ============================================================

#[derive(Debug)]
pub struct StartShellTool;

#[async_trait::async_trait]
impl Tool for StartShellTool {
    fn get_name(&self) -> String {
        "start_shell".to_string()
    }

    fn get_description(&self) -> String {
        "Start a sandboxed shell session for code execution. Creates an isolated container with the specified language runtime. Must be called before execute_shell. The session persists across multiple execute_shell calls until stop_shell is called.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "StartShellInput",
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["bash", "python", "javascript"],
                    "description": "Programming language for the session. Determines the interpreter: bash -> bash, python -> python3, javascript -> node."
                },
                "image": {
                    "type": "string",
                    "description": "Container image to use (optional, defaults to standard image with common packages pre-installed)"
                },
                "memory_mb": {
                    "type": "integer",
                    "description": "Memory limit in MB (optional, default: 256)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Session timeout in seconds (optional, default: 300)"
                }
            },
            "additionalProperties": false
        })
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(
            r#"
Start a Python session:
{"language": "python"}

Start a bash session with more memory:
{"language": "bash", "memory_mb": 512}

Start a JavaScript/Node.js session:
{"language": "javascript"}
"#
            .to_string(),
        )
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("StartShellTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for StartShellTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Check if there's already an active session
        if let Some(existing_id) = get_shell_session_id(&context).await? {
            return Ok(vec![Part::Data(json!({
                "session_id": existing_id,
                "message": "Shell session already active. Use execute_shell to run commands, or stop_shell to terminate and start a new one."
            }))]);
        }

        let input = tool_call.input;
        let overrides = get_shell_overrides(&context);

        let language = input
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Resolve image: agent input > shell overrides > env var by language
        let image = input
            .get("image")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or(overrides.image.clone())
            .or_else(|| match language.as_deref() {
                Some("python") => std::env::var("BROWSR_PYTHON_IMAGE").ok(),
                Some("javascript") => std::env::var("BROWSR_NODE_IMAGE").ok(),
                _ => None,
            });

        let memory_mb = input
            .get("memory_mb")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .or(overrides.memory_mb);

        // Collect env vars from context (injected by inject_connection_env, etc.)
        let env_vars = {
            let vars = context.env_vars.read().await;
            if vars.is_empty() {
                None
            } else {
                Some(vars.clone())
            }
        };

        let request = ShellCreateSessionRequest {
            environment_id: overrides.environment_id.clone(),
            image,
            memory_mb,
            disk_mb: overrides.disk_mb,
            cpu_cores: overrides.cpu_cores,
            language,
            timeout_secs: input
                .get("timeout_secs")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .or(overrides.timeout_secs),
            tools: overrides.tools,
            tools_endpoint: overrides.tools_endpoint,
            tool_timeout_secs: overrides.tool_timeout_secs,
            setup_script: overrides.setup_script,
            cache_paths: overrides.cache_paths,
            network_access: overrides.network_access,
            env_vars,
            ..Default::default()
        };

        let client = BrowsrShellClient::from_env();
        let response = client.create_session(&request).await?;

        // Store session ID in session store
        set_shell_session_id(&context, &response.session_id).await?;

        tracing::info!(
            "[start_shell] Created session: {} (env_vars={}, tools={:?})",
            response.session_id,
            request.env_vars.as_ref().map(|v| v.len()).unwrap_or(0),
            response.tools_injected,
        );

        Ok(vec![Part::Data(json!({
            "session_id": response.session_id,
            "status": response.status,
            "message": "Shell session started. Use execute_shell to run commands."
        }))])
    }
}

// ============================================================
// ExecuteShellTool
// ============================================================

#[derive(Debug)]
pub struct ExecuteShellTool;

#[async_trait::async_trait]
impl Tool for ExecuteShellTool {
    fn get_name(&self) -> String {
        "execute_shell".to_string()
    }

    fn get_description(&self) -> String {
        "Execute code or commands in the active shell session. Requires start_shell to be called first. Returns stdout, stderr, exit code, and execution duration. The session state (variables, files, installed packages) persists between calls.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "ExecuteShellInput",
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The code or command to execute in the shell session"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Command timeout in seconds (optional, default: 30)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for command execution (optional, default: /workspace)"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(
            r#"
Run a Python calculation:
{"command": "import math\nprint(math.factorial(10))"}

Install a package and use it:
{"command": "pip install requests && python3 -c 'import requests; r = requests.get(\"https://httpbin.org/get\"); print(r.status_code)'", "timeout_secs": 60}

Run bash commands:
{"command": "ls -la /workspace && echo 'Hello from shell'"}

Multi-line Python with data processing:
{"command": "data = [1, 2, 3, 4, 5]\nresult = sum(x**2 for x in data)\nprint(f'Sum of squares: {result}')"}

Write and execute a script file:
{"command": "cat > /workspace/analyze.py << 'EOF'\nimport json\ndata = {'key': 'value', 'count': 42}\nprint(json.dumps(data, indent=2))\nEOF\npython3 /workspace/analyze.py"}
"#
            .to_string(),
        )
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("ExecuteShellTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for ExecuteShellTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        // Get session ID from session store
        let session_id = get_shell_session_id(&context).await?.ok_or_else(|| {
            AgentError::ToolExecution(
                "No active shell session. Call start_shell first to create a session.".to_string(),
            )
        })?;

        let input = tool_call.input;
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'command' parameter".to_string()))?;

        let request = ShellExecRequest {
            session_id: session_id.clone(),
            command: command.to_string(),
            timeout_secs: input
                .get("timeout_secs")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            working_dir: input
                .get("working_dir")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        let client = BrowsrShellClient::from_env();
        let response = client.exec(&request).await?;

        let result = &response.result;
        tracing::debug!(
            "[execute_shell] session={} exit_code={:?} duration={:?}ms",
            response.session_id,
            result.exit_code,
            result.duration_ms,
        );

        Ok(vec![Part::Data(serde_json::to_value(&response).map_err(
            |e| AgentError::ToolExecution(format!("Failed to serialize: {}", e)),
        )?)])
    }
}

// ============================================================
// StopShellTool
// ============================================================

#[derive(Debug)]
pub struct StopShellTool;

#[async_trait::async_trait]
impl Tool for StopShellTool {
    fn get_name(&self) -> String {
        "stop_shell".to_string()
    }

    fn get_description(&self) -> String {
        "Stop and clean up the active shell session. Terminates the container, frees resources, and clears session state. Must be called when code execution is complete to avoid resource leaks.".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "StopShellInput",
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn get_tool_examples(&self) -> Option<String> {
        Some(
            r#"
Stop the current shell session:
{}
"#
            .to_string(),
        )
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("StopShellTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for StopShellTool {
    async fn execute_with_executor_context(
        &self,
        _tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let session_id = match get_shell_session_id(&context).await? {
            Some(id) => id,
            None => {
                return Ok(vec![Part::Data(json!({
                    "message": "No active shell session to stop."
                }))]);
            }
        };

        let client = BrowsrShellClient::from_env();
        client.destroy_session(&session_id).await?;

        // Clear session ID from store
        clear_shell_session_id(&context).await?;

        tracing::info!("[stop_shell] Destroyed shell session: {}", session_id);

        Ok(vec![Part::Data(json!({
            "session_id": session_id,
            "message": "Shell session stopped and resources cleaned up."
        }))])
    }
}
