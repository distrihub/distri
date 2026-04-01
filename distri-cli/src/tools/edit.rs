use std::path::Path;

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct EditParams {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

/// Register the `Edit` tool for exact string replacements in files.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Edit".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: EditParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Edit parameters: {}", e))?;

                let path = resolve_path(&workspace, &params.file_path);

                let content = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;

                // Count occurrences
                let count = content.matches(&params.old_string).count();

                if count == 0 {
                    anyhow::bail!(
                        "old_string not found in {}. Make sure it matches exactly.",
                        params.file_path
                    );
                }

                if count > 1 && !params.replace_all {
                    anyhow::bail!(
                        "old_string found {} times in {}. Use replace_all: true to replace all, or provide more context to make it unique.",
                        count,
                        params.file_path
                    );
                }

                let new_content = if params.replace_all {
                    content.replace(&params.old_string, &params.new_string)
                } else {
                    // Replace only the first occurrence
                    content.replacen(&params.old_string, &params.new_string, 1)
                };

                tokio::fs::write(&path, &new_content)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to write {}: {}", path.display(), e))?;

                let response = json!({
                    "file_path": params.file_path,
                    "replacements": if params.replace_all { count } else { 1 },
                    "success": true,
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Edit".to_string(),
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
