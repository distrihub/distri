//! `Bash`, `Read`, `Write` and `Edit`, run inside the sandbox bound to the
//! run's thread.
//!
//! Same names, schemas and output as distri-cli's local tools (both use
//! [`distri_types::coding_tools`]). Before each call the tool asks the
//! [`SandboxBinder`] for the thread's sandbox, which resumes or recreates it
//! when needed. Every `Bash` call is a fresh process started in the
//! binding's working directory; the directory it ends in is saved for the
//! next call.

use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use distri_types::coding_tools::{self, BashParams, EditParams, ReadParams, WriteParams};
use distri_types::{FileType, Part, Tool, ToolCall, ToolContext};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::sandbox::{BoundSandbox, ExecRequest, SandboxBinder, SandboxError, WORKSPACE_DIR};
use crate::tools::{DynExecutorTool, ExecutorContextTool};
use crate::AgentError;

const CWD_MARKER: &str = "__DISTRI_SANDBOX_CWD__=";

/// The four tools, wrapped so the executor can hand them its context.
pub fn sandbox_tools(binder: Arc<dyn SandboxBinder>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(DynExecutorTool::new(Arc::new(SandboxBashTool {
            binder: binder.clone(),
        }))),
        Arc::new(DynExecutorTool::new(Arc::new(SandboxReadTool {
            binder: binder.clone(),
        }))),
        Arc::new(DynExecutorTool::new(Arc::new(SandboxWriteTool {
            binder: binder.clone(),
        }))),
        Arc::new(DynExecutorTool::new(Arc::new(SandboxEditTool {
            binder: binder.clone(),
        }))),
        Arc::new(DynExecutorTool::new(Arc::new(SandboxSaveArtifactTool {
            binder,
        }))),
    ]
}

/// Tools a run with the sandbox does not keep: the sandbox's own version
/// replaces them (`save_artifact` reads its `path` from the sandbox).
pub const REPLACED_TOOLS: [&str; 1] = ["save_artifact"];

/// Names of the sandbox tools, for de-duplication against an agent's own.
pub const SANDBOX_TOOL_NAMES: [&str; 4] = [
    coding_tools::BASH,
    coding_tools::READ,
    coding_tools::WRITE,
    coding_tools::EDIT,
];

fn params<T: DeserializeOwned>(tool: &str, call: &ToolCall) -> Result<T, AgentError> {
    serde_json::from_value(call.input.clone())
        .map_err(|e| AgentError::ToolExecution(format!("invalid {tool} parameters: {e}")))
}

fn sandbox_err(e: SandboxError) -> AgentError {
    AgentError::ToolExecution(e.to_string())
}

async fn bind(
    binder: &Arc<dyn SandboxBinder>,
    ctx: &ExecutorContext,
) -> Result<BoundSandbox, AgentError> {
    binder
        .ensure(ctx)
        .await
        .map_err(|e| AgentError::ToolExecution(format!("sandbox unavailable: {e}")))
}

/// Absolute paths stay; relative ones are taken from the binding's cwd.
fn resolve_path(cwd: &str, file_path: &str) -> String {
    if file_path.starts_with('/') {
        file_path.to_string()
    } else {
        format!(
            "{}/{}",
            cwd.trim_end_matches('/'),
            file_path.trim_start_matches("./")
        )
    }
}

/// Wrap a command so the shell reports the directory it ended in.
fn wrap_command(command: &str) -> String {
    format!("{command}\n__distri_rc=$?\nprintf '\\n{CWD_MARKER}%s\\n' \"$PWD\"\nexit $__distri_rc")
}

/// Split the cwd marker off stdout.
fn split_cwd(stdout: &str) -> (String, Option<String>) {
    match stdout.rfind(CWD_MARKER) {
        Some(at) => {
            let rest = &stdout[at + CWD_MARKER.len()..];
            let cwd = rest.lines().next().unwrap_or("").trim().to_string();
            let mut out = stdout[..at].to_string();
            // Drop the blank line the marker's leading newline made.
            if out.ends_with("\n\n") {
                out.pop();
            } else if out == "\n" {
                out.clear();
            }
            (out, (!cwd.is_empty()).then_some(cwd))
        }
        None => (stdout.to_string(), None),
    }
}

macro_rules! tool_basics {
    ($name:expr, $desc:expr, $params:expr, $prompt:expr, $concurrent:expr) => {
        fn get_name(&self) -> String {
            $name.to_string()
        }
        fn get_description(&self) -> String {
            $desc.to_string()
        }
        fn get_parameters(&self) -> serde_json::Value {
            $params
        }
        fn prompt(&self) -> Option<String> {
            Some($prompt.to_string())
        }
        fn concurrency_safe(&self) -> bool {
            $concurrent
        }
        fn needs_executor_context(&self) -> bool {
            true
        }
    };
}

#[derive(Debug)]
pub struct SandboxBashTool {
    binder: Arc<dyn SandboxBinder>,
}

#[async_trait::async_trait]
impl Tool for SandboxBashTool {
    tool_basics!(
        coding_tools::BASH,
        coding_tools::bash_description(),
        coding_tools::bash_parameters(),
        coding_tools::BASH_PROMPT,
        false
    );

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "{} needs the executor context",
            self.get_name()
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SandboxBashTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let p: BashParams = params(coding_tools::BASH, &tool_call)?;
        let bound = bind(&self.binder, &context).await?;
        let started = std::time::Instant::now();
        let out = bound
            .provider
            .exec(
                &bound.sandbox_id,
                &ExecRequest {
                    command: wrap_command(&p.command),
                    cwd: Some(bound.cwd.clone()),
                    env: Default::default(),
                    timeout: Some(Duration::from_millis(p.timeout_ms())),
                },
                None,
            )
            .await
            .map_err(sandbox_err)?;
        let (stdout, new_cwd) = split_cwd(&out.stdout);
        if let Some(cwd) = new_cwd {
            if cwd != bound.cwd && cwd.starts_with(WORKSPACE_DIR) {
                if let Err(e) = self.binder.set_cwd(&context, &cwd).await {
                    tracing::warn!(error = %e, "could not save sandbox cwd");
                }
            }
        }
        tracing::info!(
            sandbox_id = %bound.sandbox_id,
            thread_id = %context.thread_id,
            exit_code = out.exit_code,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "sandbox Bash"
        );
        if out.timed_out {
            return Err(AgentError::ToolExecution(format!(
                "command timed out after {}ms: {}",
                p.timeout_ms(),
                p.command
            )));
        }
        Ok(vec![Part::Data(coding_tools::bash_result(
            &stdout,
            &out.stderr,
            out.exit_code,
        ))])
    }
}

#[derive(Debug)]
pub struct SandboxReadTool {
    binder: Arc<dyn SandboxBinder>,
}

#[async_trait::async_trait]
impl Tool for SandboxReadTool {
    tool_basics!(
        coding_tools::READ,
        coding_tools::read_description(),
        coding_tools::read_parameters(),
        coding_tools::READ_PROMPT,
        true
    );

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "{} needs the executor context",
            self.get_name()
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SandboxReadTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let p: ReadParams = params(coding_tools::READ, &tool_call)?;
        let bound = bind(&self.binder, &context).await?;
        let path = resolve_path(&bound.cwd, &p.file_path);
        let content = bound
            .provider
            .read_file(&bound.sandbox_id, &path)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("failed to read {path}: {e}")))?;

        if let Some(mime) = coding_tools::image_mime_for_path(&path) {
            let name = path.rsplit('/').next().map(|s| s.to_string());
            return Ok(vec![
                Part::Data(json!({
                    "file_path": p.file_path,
                    "size_bytes": content.len(),
                    "mime_type": mime,
                    "name": name,
                    "kind": "image",
                })),
                Part::Image(FileType::Bytes {
                    bytes: STANDARD.encode(&content),
                    mime_type: mime.to_string(),
                    name,
                }),
            ]);
        }
        let text = String::from_utf8_lossy(&content);
        Ok(vec![Part::Data(coding_tools::read_text_result(
            &p.file_path,
            &text,
            p.offset,
            p.limit,
        ))])
    }
}

#[derive(Debug)]
pub struct SandboxWriteTool {
    binder: Arc<dyn SandboxBinder>,
}

#[async_trait::async_trait]
impl Tool for SandboxWriteTool {
    tool_basics!(
        coding_tools::WRITE,
        coding_tools::write_description(),
        coding_tools::write_parameters(),
        coding_tools::WRITE_PROMPT,
        false
    );

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "{} needs the executor context",
            self.get_name()
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SandboxWriteTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let p: WriteParams = params(coding_tools::WRITE, &tool_call)?;
        let bound = bind(&self.binder, &context).await?;
        let path = resolve_path(&bound.cwd, &p.file_path);
        bound
            .provider
            .write_file(&bound.sandbox_id, &path, p.content.as_bytes())
            .await
            .map_err(|e| AgentError::ToolExecution(format!("failed to write {path}: {e}")))?;
        Ok(vec![Part::Data(coding_tools::write_result(
            &p.file_path,
            p.content.len(),
        ))])
    }
}

#[derive(Debug)]
pub struct SandboxEditTool {
    binder: Arc<dyn SandboxBinder>,
}

#[async_trait::async_trait]
impl Tool for SandboxEditTool {
    tool_basics!(
        coding_tools::EDIT,
        coding_tools::edit_description(),
        coding_tools::edit_parameters(),
        coding_tools::EDIT_PROMPT,
        false
    );

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "{} needs the executor context",
            self.get_name()
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SandboxEditTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let p: EditParams = params(coding_tools::EDIT, &tool_call)?;
        let bound = bind(&self.binder, &context).await?;
        let path = resolve_path(&bound.cwd, &p.file_path);
        let content = bound
            .provider
            .read_file(&bound.sandbox_id, &path)
            .await
            .map_err(|e| AgentError::ToolExecution(format!("failed to read {path}: {e}")))?;
        let content = String::from_utf8(content)
            .map_err(|_| AgentError::ToolExecution(format!("{path} is not UTF-8 text")))?;
        let (new_content, replacements) =
            coding_tools::apply_edit(&content, &p).map_err(AgentError::ToolExecution)?;
        bound
            .provider
            .write_file(&bound.sandbox_id, &path, new_content.as_bytes())
            .await
            .map_err(|e| AgentError::ToolExecution(format!("failed to write {path}: {e}")))?;
        Ok(vec![Part::Data(coding_tools::edit_result(
            &p.file_path,
            replacements,
        ))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cwd_marker() {
        let (out, cwd) = split_cwd("hello\n\n__DISTRI_SANDBOX_CWD__=/workspace/a\n");
        assert_eq!(out, "hello\n");
        assert_eq!(cwd.as_deref(), Some("/workspace/a"));
        let (out, cwd) = split_cwd("\n__DISTRI_SANDBOX_CWD__=/workspace\n");
        assert_eq!(out, "");
        assert_eq!(cwd.as_deref(), Some("/workspace"));
        let (out, cwd) = split_cwd("no marker\n");
        assert_eq!(out, "no marker\n");
        assert!(cwd.is_none());
    }

    #[test]
    fn resolves_paths() {
        assert_eq!(resolve_path("/workspace", "fib.py"), "/workspace/fib.py");
        assert_eq!(resolve_path("/workspace/", "./a/b"), "/workspace/a/b");
        assert_eq!(resolve_path("/workspace", "/etc/x"), "/etc/x");
    }
}

/// `save_artifact` for a run with a sandbox: a `path` is read from the
/// thread's sandbox (relative to its working directory), `content` is saved
/// as given.
#[derive(Debug)]
pub struct SandboxSaveArtifactTool {
    binder: Arc<dyn SandboxBinder>,
}

#[async_trait::async_trait]
impl Tool for SandboxSaveArtifactTool {
    fn get_name(&self) -> String {
        crate::tools::save_artifact::SaveArtifactTool.get_name()
    }
    fn get_description(&self) -> String {
        crate::tools::save_artifact::SaveArtifactTool.get_description()
    }
    fn get_parameters(&self) -> serde_json::Value {
        crate::tools::save_artifact::SaveArtifactTool.get_parameters()
    }
    fn needs_executor_context(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "{} needs the executor context",
            self.get_name()
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for SandboxSaveArtifactTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        use crate::tools::save_artifact::{missing_file, store_artifact, ArtifactRequest};
        let req = ArtifactRequest::parse(&tool_call)?;
        let bytes = match (&req.content, &req.path) {
            (Some(content), _) => content.as_bytes().to_vec(),
            (None, Some(path)) => {
                let bound = bind(&self.binder, &context).await?;
                let full = resolve_path(&bound.cwd, path);
                bound
                    .provider
                    .read_file(&bound.sandbox_id, &full)
                    .await
                    .map_err(|e| missing_file(&full, &format!("could not be read ({e})")))?
            }
            (None, None) => unreachable!("parse requires path or content"),
        };
        store_artifact(&context, req, bytes).await
    }
}
