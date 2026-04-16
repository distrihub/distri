use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use base64::{engine::general_purpose, Engine as _};
use browsr_types::ShellExecRequest;
use distri_types::{AgentEventType, FileMetadata, Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

use super::shell::get_shell_session_id;
use browsr_client::BrowsrClient;

/// Tool for sending chart/image files from a shell session to the user.
///
/// The tool reads the image file server-side (no base64 through the LLM),
/// saves it as a persistent artifact, and emits a `MediaGenerated` event
/// so all channels (Telegram, Web, CLI) render the image immediately.
#[derive(Debug)]
pub struct RenderChartTool;

#[async_trait::async_trait]
impl Tool for RenderChartTool {
    fn get_name(&self) -> String {
        "render_chart".to_string()
    }

    fn get_description(&self) -> String {
        "Send a chart or image file from the active shell session to the user. The file is saved as an artifact and displayed immediately on all channels. Call this after generating an image (e.g. matplotlib chart) saved in the shell workspace.".to_string()
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
                    "description": "Absolute path to the image file in the shell workspace (e.g. \"/workspace/chart.png\")"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional caption text for the image"
                },
                "filename": {
                    "type": "string",
                    "description": "Optional filename for the artifact (e.g. \"sales_chart.png\"). Defaults to the basename of path."
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
        Err(anyhow::anyhow!(
            "RenderChartTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for RenderChartTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let path = tool_call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing 'path' parameter".to_string())
            })?;

        let caption: Option<&str> = tool_call
            .input
            .get("caption")
            .and_then(|v| v.as_str());

        let filename: String = tool_call
            .input
            .get("filename")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "chart.png".to_string())
            });

        // 1. Read the image file — try browsr shell session first, fall back
        //    to local filesystem (for agents using Bash instead of start_shell).
        let base64_str = if let Ok(Some(session_id)) = get_shell_session_id(&context).await {
            // Read via BrowsrClient shell session (server-side, no base64 through LLM)
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
            // No shell session — read from local filesystem
            let raw_bytes = std::fs::read(path).map_err(|e| {
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
            general_purpose::STANDARD.encode(&raw_bytes)
        };

        // 3. Decode to verify and get size
        let bytes = general_purpose::STANDARD
            .decode(&base64_str)
            .map_err(|e| AgentError::ToolExecution(format!("Invalid base64 data: {}", e)))?;
        let size = bytes.len() as u64;

        // 4. Infer mime type from filename extension
        let mime_type = mime_from_filename(&filename);

        // 5. Save as artifact via ArtifactWrapper
        let artifact_path = if let Ok(orchestrator) = context.get_orchestrator() {
            let base_path =
                distri_filesystem::ArtifactWrapper::task_namespace(&context.thread_id, &context.task_id);
            match orchestrator
                .session_filesystem
                .create_artifact_wrapper(base_path)
                .await
            {
                Ok(wrapper) => {
                    // artifact_path is {prefix_path}/content/{filename}
                    let ap = format!("{}/content/{}", wrapper.prefix_path(), filename);
                    // Save the base64 content as text (images are stored as base64 strings)
                    if let Err(e) = wrapper.save_artifact(&filename, &base64_str).await {
                        tracing::warn!("Failed to save chart artifact: {}", e);
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

        // 6. Emit MediaGenerated event for immediate rendering on all channels
        context
            .emit(AgentEventType::MediaGenerated {
                data: base64_str,
                mime_type: mime_type.to_string(),
                filename: Some(filename.clone()),
                size: Some(size),
                artifact_path: artifact_path.clone(),
            })
            .await;

        tracing::info!(
            path = path,
            filename = %filename,
            size = size,
            mime = %mime_type,
            "render_chart: media generated"
        );

        // 7. Return Part::Artifact so the LLM knows the file was saved
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
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        _ => "image/png",
    }
}
