use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use base64::{engine::general_purpose, Engine as _};
use browsr_types::ShellExecRequest;
use distri_types::{FileMetadata, Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

use super::shell::get_shell_session_id;
use browsr_client::BrowsrClient;

/// Tool for saving any file from the agent's workspace as a persistent
/// artifact. The file is persisted in the artifact store and returned as
/// `Part::Artifact(FileMetadata)` in the tool response — each channel renders
/// it based on MIME type (images inline, markdown/JSON as previews, etc.).
#[derive(Debug)]
pub struct SaveArtifactTool;

#[async_trait::async_trait]
impl Tool for SaveArtifactTool {
    fn get_name(&self) -> String {
        "save_artifact".to_string()
    }

    fn get_description(&self) -> String {
        "Save an artifact to share with the user. Provide EITHER `path` (reads a file from the shell workspace or local filesystem) OR `content` (a string to save directly). The artifact is persisted and rendered by each channel according to its MIME type (images inline, markdown/JSON as previews, etc.).".to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to save (e.g. \"/workspace/chart.png\"). Mutually exclusive with `content`."
                },
                "content": {
                    "type": "string",
                    "description": "Inline string content to save. Mutually exclusive with `path`. Requires `filename` to be set so the MIME type can be inferred."
                },
                "filename": {
                    "type": "string",
                    "description": "Filename (with extension) to save the artifact as. Optional when `path` is given (defaults to basename of path). Required when `content` is given."
                },
                "caption": {
                    "type": "string",
                    "description": "Optional description shown alongside the artifact."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("SaveArtifactTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SaveArtifactTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let path = tool_call.input.get("path").and_then(|v| v.as_str());
        let content_field = tool_call.input.get("content").and_then(|v| v.as_str());
        let caption: Option<&str> = tool_call.input.get("caption").and_then(|v| v.as_str());

        // Enforce exactly one of path/content
        if path.is_some() && content_field.is_some() {
            return Err(AgentError::ToolExecution(
                "Provide either 'path' or 'content', not both".to_string(),
            ));
        }

        // Resolve filename: explicit override > basename(path) > required-for-content
        let filename: String = match tool_call
            .input
            .get("filename")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
        {
            Some(f) => f,
            None => match path {
                Some(p) => std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "artifact.bin".to_string()),
                None => {
                    return Err(AgentError::ToolExecution(
                        "'filename' is required when using 'content' mode".to_string(),
                    ));
                }
            },
        };

        // Acquire base64-encoded bytes in both modes
        let (base64_str, raw_size) = if let Some(content) = content_field {
            let raw_bytes = content.as_bytes();
            let size = raw_bytes.len() as u64;
            (general_purpose::STANDARD.encode(raw_bytes), size)
        } else if let Some(path) = path {
            if let Ok(Some(session_id)) = get_shell_session_id(&context).await {
                let client = BrowsrClient::from_env();
                let response = client
                    .shell_exec(ShellExecRequest {
                        session_id: session_id.clone(),
                        command: format!("base64 -w0 {}", shell_quote(path)),
                        timeout_secs: Some(30),
                        working_dir: None,
                    })
                    .await
                    .map_err(|e| {
                        AgentError::ToolExecution(format!("Failed to read file from shell: {}", e))
                    })?;

                if response.result.exit_code != Some(0) {
                    return Err(AgentError::ToolExecution(format!(
                        "Failed to read file '{}': {}",
                        path, response.result.stderr
                    )));
                }

                let s = response.result.stdout.trim().to_string();
                if s.is_empty() {
                    return Err(AgentError::ToolExecution(format!(
                        "File '{}' is empty or does not exist",
                        path
                    )));
                }
                let decoded = general_purpose::STANDARD
                    .decode(&s)
                    .map_err(|e| AgentError::ToolExecution(format!("Invalid base64 data: {}", e)))?;
                let size = decoded.len() as u64;
                (s, size)
            } else {
                let raw_bytes = tokio::fs::read(path).await.map_err(|e| {
                    AgentError::ToolExecution(format!(
                        "No active shell session and local file '{}' not readable: {}",
                        path, e
                    ))
                })?;
                if raw_bytes.is_empty() {
                    return Err(AgentError::ToolExecution(format!(
                        "File '{}' is empty",
                        path
                    )));
                }
                let size = raw_bytes.len() as u64;
                (general_purpose::STANDARD.encode(&raw_bytes), size)
            }
        } else {
            return Err(AgentError::ToolExecution(
                "Must provide either 'path' or 'content'".to_string(),
            ));
        };

        let mime_type = mime_from_filename(&filename);

        // Persist to artifact store
        let artifact_path = if let Ok(orchestrator) = context.get_orchestrator() {
            let base_path = distri_filesystem::ArtifactWrapper::task_namespace(
                &context.thread_id,
                &context.task_id,
            );
            match orchestrator
                .session_filesystem
                .create_artifact_wrapper(base_path)
                .await
            {
                Ok(wrapper) => {
                    let ap = format!("{}/content/{}", wrapper.prefix_path(), filename);
                    if let Err(e) = wrapper.save_artifact(&filename, &base64_str).await {
                        tracing::warn!("Failed to save artifact: {}", e);
                    }
                    Some(ap)
                }
                Err(e) => {
                    tracing::warn!("Failed to create artifact wrapper: {}", e);
                    None
                }
            }
        } else {
            None
        };

        tracing::info!(
            filename = %filename,
            size = raw_size,
            mime = %mime_type,
            artifact_path = ?artifact_path,
            mode = if path.is_some() { "path" } else { "content" },
            "save_artifact: artifact saved"
        );

        let metadata = FileMetadata {
            file_id: filename.clone(),
            relative_path: artifact_path.unwrap_or_default(),
            size: raw_size,
            content_type: Some(mime_type.to_string()),
            original_filename: Some(filename),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            checksum: None,
            stats: None,
            preview: caption.map(|c| c.to_string()),
        };

        Ok(vec![Part::Artifact(metadata)])
    }
}

/// Infer MIME type from filename extension.
fn mime_from_filename(filename: &str) -> &'static str {
    match std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("md") | Some("markdown") => "text/markdown",
        Some("html") | Some("htm") => "text/html",
        Some("txt") => "text/plain",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Escape a path for safe inclusion in a POSIX shell command.
/// Wraps in single quotes; any embedded single quote becomes `'\''`.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}
