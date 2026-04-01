use std::path::Path;

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
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

/// Register the `Read` tool for reading local files with line numbers.
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
