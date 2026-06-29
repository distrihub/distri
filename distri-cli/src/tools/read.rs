use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, FileType, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct ReadParams {
    file_path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

const MAX_LINES: usize = 2000;

/// Register the `Read` tool for reading local files. Text files come back
/// as line-numbered text in a `Part::Data`; images come back as a small
/// `Part::Data` descriptor PLUS a `Part::Image` so the multimodal model
/// can see them on the next turn.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Read".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: ReadParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Read parameters: {}", e))?;

                let path = resolve_path(&workspace, &params.file_path);

                let content = tokio::fs::read(&path)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;

                // If the file extension says image (or we sniff one),
                // route through the image path: small JSON descriptor +
                // the image bytes as a vision part. The model sees the
                // image directly on the next turn via the LLM client's
                // tool-result image handling.
                if let Some(mime) = image_mime_for_path(&path) {
                    let b64 = STANDARD.encode(&content);
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string());
                    let descriptor = json!({
                        "file_path": params.file_path,
                        "size_bytes": content.len(),
                        "mime_type": mime,
                        "name": name,
                        "kind": "image",
                    });
                    return Ok(ToolResponse::from_parts(
                        call.tool_call_id.clone(),
                        "Read".to_string(),
                        vec![
                            Part::Data(descriptor),
                            Part::Image(FileType::Bytes {
                                bytes: b64,
                                mime_type: mime.to_string(),
                                name,
                            }),
                        ],
                    ));
                }

                let text = String::from_utf8_lossy(&content);
                let all_lines: Vec<&str> = text.lines().collect();
                let total_lines = all_lines.len();

                let offset = params.offset.unwrap_or(0);
                let limit = params.limit.unwrap_or(MAX_LINES).min(MAX_LINES);

                let start = offset.min(total_lines);
                let end = (start + limit).min(total_lines);
                let selected = &all_lines[start..end];

                // Format with line numbers (cat -n style)
                let numbered: String = selected
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{:>4}\t{}", start + i + 1, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                let response = json!({
                    "content": numbered,
                    "file_path": params.file_path,
                    "total_lines": total_lines,
                    "lines_read": end - start,
                    "truncated": end < total_lines,
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Read".to_string(),
                    vec![Part::Data(response)],
                ))
            }
        },
    );
}

fn resolve_path(workspace: &Path, file_path: &str) -> std::path::PathBuf {
    let p = Path::new(file_path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(file_path)
    }
}

/// Return the MIME type for a path if its extension says image.
/// Returns `None` for everything else (which keeps the existing text path).
fn image_mime_for_path(path: &Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}
