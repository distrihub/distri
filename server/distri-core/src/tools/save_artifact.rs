use crate::agent::ExecutorContext;
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use base64::{engine::general_purpose, Engine as _};
use distri_types::{FileMetadata, Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::sync::Arc;

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
        "Save an artifact to share with the user. Provide EITHER `path` (reads a file from the sandbox when the run has one, else the local filesystem) OR `content` (a string to save directly). The artifact is persisted and rendered by each channel according to its MIME type (images inline, markdown/JSON as previews, etc.).".to_string()
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

/// What a `save_artifact` call asks for, before its bytes are read.
pub(crate) struct ArtifactRequest {
    pub path: Option<String>,
    pub content: Option<String>,
    pub filename: String,
    pub caption: Option<String>,
}

impl ArtifactRequest {
    pub(crate) fn parse(tool_call: &ToolCall) -> Result<Self, AgentError> {
        let get = |k: &str| {
            tool_call
                .input
                .get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        let (path, content) = (get("path"), get("content"));
        if path.is_some() && content.is_some() {
            return Err(AgentError::ToolExecution(
                "Provide either 'path' or 'content', not both".to_string(),
            ));
        }
        if path.is_none() && content.is_none() {
            return Err(AgentError::ToolExecution(
                "Must provide either 'path' or 'content'".to_string(),
            ));
        }
        // Explicit override > basename(path) > required-for-content.
        let filename = match (get("filename"), path.as_deref()) {
            (Some(f), _) => f,
            (None, Some(p)) => std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "artifact.bin".to_string()),
            (None, None) => {
                return Err(AgentError::ToolExecution(
                    "'filename' is required when using 'content' mode".to_string(),
                ));
            }
        };
        Ok(Self {
            path,
            content,
            filename,
            caption: get("caption"),
        })
    }
}

/// The error for a file that is missing or empty, worded so the model
/// checks that its code really wrote the file.
pub(crate) fn missing_file(path: &str, why: &str) -> AgentError {
    AgentError::ToolExecution(format!(
        "save_artifact: file '{path}' {why}. Verify with `ls -la {path}` and ensure your code actually writes the file (e.g. matplotlib needs `plt.savefig('{path}')`: printing the path is not enough)."
    ))
}

#[async_trait::async_trait]
impl ExecutorContextTool for SaveArtifactTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let req = ArtifactRequest::parse(&tool_call)?;
        let bytes = match (&req.content, &req.path) {
            (Some(content), _) => content.as_bytes().to_vec(),
            (None, Some(path)) => tokio::fs::read(path)
                .await
                .map_err(|e| missing_file(path, &format!("does not exist ({e})")))?,
            (None, None) => unreachable!("parse requires path or content"),
        };
        store_artifact(&context, req, bytes).await
    }
}

/// Persist `bytes` as the call's artifact and return it as a part.
pub(crate) async fn store_artifact(
    context: &ExecutorContext,
    req: ArtifactRequest,
    bytes: Vec<u8>,
) -> Result<Vec<Part>, AgentError> {
    if let (Some(path), true) = (&req.path, bytes.is_empty()) {
        return Err(missing_file(path, "exists but is empty (0 bytes)"));
    }
    let raw_size = bytes.len() as u64;
    let base64_str = general_purpose::STANDARD.encode(&bytes);
    let filename = req.filename;
    let mime_type = mime_from_filename(&filename);

    // Persist to artifact store
    let artifact_path = if let Ok(orchestrator) = context.get_orchestrator() {
        let base_path = distri_filesystem::ArtifactWrapper::task_namespace(
            &context.thread_id,
            &context.task_id,
        );
        let wrapper = match orchestrator.session_filesystem() {
            Ok(fs) => fs.create_artifact_wrapper(base_path).await,
            Err(e) => Err(e),
        };
        match wrapper {
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
        mode = if req.path.is_some() { "path" } else { "content" },
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
        preview: req.caption,
    };

    Ok(vec![Part::Artifact(metadata)])
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
