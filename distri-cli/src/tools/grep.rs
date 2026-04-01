use std::path::Path;
use std::process::Stdio;

use distri::ExternalToolRegistry;
use distri_types::{AgentEvent, Part, ToolCall, ToolResponse};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct GrepParams {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    output_mode: Option<String>,
    #[serde(default, rename = "-B")]
    before_context: Option<usize>,
    #[serde(default, rename = "-A")]
    after_context: Option<usize>,
    #[serde(default, rename = "-C")]
    context_alias: Option<usize>,
    #[serde(default)]
    context: Option<usize>,
    #[serde(default, rename = "-n")]
    line_numbers: Option<bool>,
    #[serde(default, rename = "-i")]
    case_insensitive: Option<bool>,
    #[serde(default, rename = "type")]
    file_type: Option<String>,
    #[serde(default)]
    head_limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    multiline: Option<bool>,
}

const DEFAULT_HEAD_LIMIT: usize = 250;

/// Register the `Grep` tool for ripgrep-based content search.
pub fn register(registry: &ExternalToolRegistry, agent_id: &str, workspace_root: &Path) {
    let workspace = workspace_root.to_path_buf();

    registry.register(
        agent_id.to_string(),
        "Grep".to_string(),
        move |call: ToolCall, _event: AgentEvent| {
            let workspace = workspace.clone();
            async move {
                let params: GrepParams = serde_json::from_value(call.input.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Grep parameters: {}", e))?;

                let search_path = match &params.path {
                    Some(p) if !p.is_empty() => resolve_path(&workspace, p),
                    _ => workspace.clone(),
                };

                let output_mode = params
                    .output_mode
                    .as_deref()
                    .unwrap_or("files_with_matches");

                let mut args: Vec<String> = Vec::new();

                // Output mode
                match output_mode {
                    "files_with_matches" => args.push("--files-with-matches".to_string()),
                    "count" => args.push("--count".to_string()),
                    "content" | _ if output_mode == "content" => {
                        // Default content mode — show matching lines
                        let show_numbers = params.line_numbers.unwrap_or(true);
                        if show_numbers {
                            args.push("--line-number".to_string());
                        }
                    }
                    _ => args.push("--files-with-matches".to_string()),
                }

                // Context lines (only for content mode)
                if output_mode == "content" {
                    let ctx = params.context_alias.or(params.context);
                    if let Some(c) = ctx {
                        args.push(format!("--context={}", c));
                    } else {
                        if let Some(b) = params.before_context {
                            args.push(format!("--before-context={}", b));
                        }
                        if let Some(a) = params.after_context {
                            args.push(format!("--after-context={}", a));
                        }
                    }
                }

                if params.case_insensitive.unwrap_or(false) {
                    args.push("--ignore-case".to_string());
                }

                if params.multiline.unwrap_or(false) {
                    args.push("--multiline".to_string());
                    args.push("--multiline-dotall".to_string());
                }

                if let Some(ref g) = params.glob {
                    args.push(format!("--glob={}", g));
                }

                if let Some(ref t) = params.file_type {
                    args.push(format!("--type={}", t));
                }

                // Pattern and path
                args.push("--".to_string());
                args.push(params.pattern.clone());
                args.push(search_path.to_string_lossy().to_string());

                let output = tokio::process::Command::new("rg")
                    .args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "failed to run rg (is ripgrep installed?): {}",
                            e
                        )
                    })?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Apply offset and head_limit
                let offset = params.offset.unwrap_or(0);
                let head_limit = params.head_limit.unwrap_or(DEFAULT_HEAD_LIMIT);

                let lines: Vec<&str> = stdout.lines().collect();
                let total = lines.len();

                let selected: Vec<&str> = if head_limit == 0 {
                    lines.into_iter().skip(offset).collect()
                } else {
                    lines.into_iter().skip(offset).take(head_limit).collect()
                };

                let result_text = selected.join("\n");
                let truncated = offset + selected.len() < total;

                // Make paths relative to workspace
                let result_text = make_paths_relative(&result_text, &workspace);

                let response = json!({
                    "output": result_text,
                    "total_lines": total,
                    "truncated": truncated,
                    "exit_code": output.status.code().unwrap_or(-1),
                    "stderr": if stderr.is_empty() { None } else { Some(stderr) },
                });

                Ok(ToolResponse::from_parts(
                    call.tool_call_id.clone(),
                    "Grep".to_string(),
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

fn make_paths_relative(text: &str, workspace: &Path) -> String {
    let prefix = format!("{}/", workspace.display());
    text.replace(&prefix, "")
}
