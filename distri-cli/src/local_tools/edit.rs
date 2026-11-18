use std::path::Path;

use distri::ExternalToolRegistry;
use distri_types::coding_tools::EditParams;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};

/// Register the `Edit` tool for exact string replacements in files.
pub(super) fn register(
    registry: &ExternalToolRegistry,
    agent_id: &str,
    workspace_root: &Path,
    gate: super::Gate,
) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Edit".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            let gate = gate.clone();
            async move {
                gate.authorize("Edit", &call).await?;
                let params: EditParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Edit parameters: {}", e))?;

                let path = resolve_path(&workspace, &params.file_path);

                let content = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to read {}: {}", path.display(), e))?;

                let (new_content, replacements) =
                    distri_types::coding_tools::apply_edit(&content, &params)
                        .map_err(|e| anyhow::anyhow!(e))?;

                tokio::fs::write(&path, &new_content)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to write {}: {}", path.display(), e))?;

                let response =
                    distri_types::coding_tools::edit_result(&params.file_path, replacements);

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
