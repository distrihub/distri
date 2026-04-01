use std::path::Path;

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct WriteParams {
    file_path: String,
    content: String,
}

/// Register the `Write` tool for writing/creating local files.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Write".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: WriteParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Write parameters: {}", e))?;

                let path = resolve_path(&workspace, &params.file_path);

                // Ensure parent directory exists
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        anyhow::anyhow!("failed to create directory {}: {}", parent.display(), e)
                    })?;
                }

                tokio::fs::write(&path, &params.content)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to write {}: {}", path.display(), e))?;

                let response = json!({
                    "file_path": params.file_path,
                    "bytes_written": params.content.len(),
                    "success": true,
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Write".to_string(),
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
