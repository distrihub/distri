use std::path::Path;
use std::time::Instant;

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct GlobParams {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

const MAX_RESULTS: usize = 100;

/// Register the `Glob` tool for file pattern matching.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Glob".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: GlobParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Glob parameters: {}", e))?;

                let search_dir = match &params.path {
                    Some(p) if !p.is_empty() => resolve_path(&workspace, p),
                    _ => workspace.clone(),
                };

                let start = Instant::now();

                // Build full glob pattern
                let full_pattern = format!("{}/{}", search_dir.display(), params.pattern);

                let entries = tokio::task::spawn_blocking(move || {
                    let mut results = Vec::new();
                    if let Ok(paths) = ::glob::glob(&full_pattern) {
                        for entry in paths.flatten() {
                            results.push(entry);
                            if results.len() >= MAX_RESULTS {
                                break;
                            }
                        }
                    }
                    results
                })
                .await
                .map_err(|e| anyhow::anyhow!("glob task failed: {}", e))?;

                let duration_ms = start.elapsed().as_millis();
                let truncated = entries.len() >= MAX_RESULTS;

                // Convert to relative paths
                let filenames: Vec<String> = entries
                    .iter()
                    .filter_map(|p| {
                        p.strip_prefix(&workspace)
                            .ok()
                            .map(|r| r.to_string_lossy().to_string())
                    })
                    .collect();

                let response = json!({
                    "filenames": filenames,
                    "num_files": filenames.len(),
                    "duration_ms": duration_ms,
                    "truncated": truncated,
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Glob".to_string(),
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
