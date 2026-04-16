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
        "Save a file from the workspace as a persistent artifact. Use for any file you want to share with the user: images (PNG, JPG, SVG), documents (PDF, Markdown), data (JSON, CSV), or videos. The file is stored in the artifact store and each channel renders it appropriately.".to_string()
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
                    "description": "Absolute path to the file in the shell workspace or local filesystem (e.g. \"/workspace/chart.png\", \"/tmp/results.json\")"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional description shown alongside the artifact"
                },
                "filename": {
                    "type": "string",
                    "description": "Optional filename override. Defaults to the basename of path."
                }
            },
            "required": ["path"]
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
        let path = tool_call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'path' parameter".to_string()))?;

        let caption: Option<&str> = tool_call.input.get("caption").and_then(|v| v.as_str());

        let filename: String = tool_call
            .input
            .get("filename")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "artifact.bin".to_string())
            });

        // Read the file — try browsr shell session first, fall back to local fs
        let base64_str = if let Ok(Some(session_id)) = get_shell_session_id(&context).await {
            let client = BrowsrClient::from_env();
            let response = client
                .shell_exec(ShellExecRequest {
                    session_id: session_id.clone(),
                    command: format!("base64 -w0 {}", path),
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
            s
        } else {
            let raw_bytes = std::fs::read(path).map_err(|e| {
                AgentError::ToolExecution(format!(
                    "No active shell session and local file '{}' not readable: {}",
                    path, e
                ))
            })?;
            if raw_bytes.is_empty() {
                return Err(AgentError::ToolExecution(format!("File '{}' is empty", path)));
            }
            general_purpose::STANDARD.encode(&raw_bytes)
        };

        let bytes = general_purpose::STANDARD
            .decode(&base64_str)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid base64 data: {}", e)))?;
        let size = bytes.len() as u64;

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
            path = path,
            filename = %filename,
            size = size,
            mime = %mime_type,
            artifact_path = ?artifact_path,
            "save_artifact: artifact saved"
        );

        let metadata = FileMetadata {
            file_id: filename.clone(),
            relative_path: artifact_path.unwrap_or_default(),
            size,
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
