//! Sandboxes for agent tools.
//!
//! A [`SandboxProvider`] owns the lifecycle of isolated sandboxes (create,
//! pause, resume, delete, status) and gives tools a way to run commands and
//! move files inside one. [`opensandbox::OpenSandboxProvider`] is the first
//! implementation (OpenSandbox's lifecycle API plus its `execd` data plane).
//!
//! A [`SandboxBinder`] maps an agent run (its thread) onto one sandbox. The
//! host application owns that binding (cloud keeps it in Postgres) and makes
//! sure the sandbox is running before each tool call: running → use it,
//! paused → resume it, gone → create a new one on the same volume.
//!
//! The tools that run inside the bound sandbox live in
//! [`crate::tools::sandbox_tools`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::agent::ExecutorContext;

pub mod opensandbox;

pub use opensandbox::OpenSandboxProvider;

/// The mount point of the per-thread volume inside every sandbox.
pub const WORKSPACE_DIR: &str = "/workspace";

/// What to create.
#[derive(Debug, Clone, Default)]
pub struct SandboxSpec {
    /// Container image, e.g. `python:3.12-slim`.
    pub image: String,
    /// Named volume mounted at [`WORKSPACE_DIR`]. Created when it does not
    /// exist and kept when the sandbox is deleted, so a new sandbox on the
    /// same volume sees the same files.
    pub volume: Option<String>,
    pub env: HashMap<String, String>,
    pub metadata: HashMap<String, String>,
    /// Seconds until the provider expires the sandbox; `None` uses the
    /// provider's default.
    pub timeout_secs: Option<u64>,
}

/// Lifecycle state, normalised across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxState {
    Pending,
    Running,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    /// Terminated, failed, expired or not found: create a new one.
    Gone,
}

impl SandboxState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxState::Pending => "pending",
            SandboxState::Running => "running",
            SandboxState::Pausing => "pausing",
            SandboxState::Paused => "paused",
            SandboxState::Resuming => "resuming",
            SandboxState::Stopping => "stopping",
            SandboxState::Gone => "gone",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub id: String,
    pub state: SandboxState,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecRequest {
    /// Shell text, run with `bash -c` semantics by the provider.
    pub command: String,
    pub cwd: Option<String>,
    pub env: HashMap<String, String>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    /// `-1` when the process was killed (timeout, signal) or never reported.
    pub exit_code: i64,
    pub timed_out: bool,
}

/// One chunk of output while a command runs.
#[derive(Debug, Clone)]
pub enum ExecChunk {
    Stdout(String),
    Stderr(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox not found: {0}")]
    NotFound(String),
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("sandbox provider error: {0}")]
    Provider(String),
    #[error("sandbox not configured: {0}")]
    NotConfigured(String),
}

pub type SandboxResult<T> = Result<T, SandboxError>;

/// Lifecycle plus exec and file access for one kind of sandbox.
#[async_trait::async_trait]
pub trait SandboxProvider: Send + Sync + std::fmt::Debug {
    /// Short stable name stored with bindings (`opensandbox`).
    fn name(&self) -> &'static str;

    async fn create(&self, spec: &SandboxSpec) -> SandboxResult<SandboxInfo>;
    /// `Ok(state == Gone)` when the provider no longer knows the sandbox.
    async fn status(&self, sandbox_id: &str) -> SandboxResult<SandboxInfo>;
    /// Ask to pause; returns once the request is accepted (it may still be
    /// `Pausing`). Use [`wait_for_state`] to wait for `Paused`.
    async fn pause(&self, sandbox_id: &str) -> SandboxResult<()>;
    async fn resume(&self, sandbox_id: &str) -> SandboxResult<()>;
    /// Delete the sandbox. Its volume is kept.
    async fn delete(&self, sandbox_id: &str) -> SandboxResult<()>;
    /// Wait until commands and files can be served (after create or
    /// resume the sandbox may be `Running` before its agent answers).
    async fn wait_ready(&self, _sandbox_id: &str, _timeout: Duration) -> SandboxResult<()> {
        Ok(())
    }

    /// Run a command to completion and collect its output. When `on_chunk`
    /// is given, output is also sent there as it arrives.
    async fn exec(
        &self,
        sandbox_id: &str,
        req: &ExecRequest,
        on_chunk: Option<tokio::sync::mpsc::UnboundedSender<ExecChunk>>,
    ) -> SandboxResult<ExecOutput>;

    async fn read_file(&self, sandbox_id: &str, path: &str) -> SandboxResult<Vec<u8>>;
    /// Write a file, creating parent directories.
    async fn write_file(&self, sandbox_id: &str, path: &str, content: &[u8]) -> SandboxResult<()>;
}

/// Poll `status` until the sandbox reaches `want` (or `Gone`), up to `timeout`.
pub async fn wait_for_state(
    provider: &dyn SandboxProvider,
    sandbox_id: &str,
    want: SandboxState,
    timeout: Duration,
) -> SandboxResult<SandboxInfo> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut delay = Duration::from_millis(100);
    loop {
        let info = provider.status(sandbox_id).await?;
        if info.state == want || info.state == SandboxState::Gone {
            return Ok(info);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SandboxError::Provider(format!(
                "sandbox {sandbox_id} still {} after {:?} (wanted {})",
                info.state.as_str(),
                timeout,
                want.as_str()
            )));
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

/// The sandbox a run's tools use, after [`SandboxBinder::ensure`].
#[derive(Debug, Clone)]
pub struct BoundSandbox {
    pub provider: Arc<dyn SandboxProvider>,
    pub sandbox_id: String,
    /// Working directory for the next command (under [`WORKSPACE_DIR`]).
    pub cwd: String,
}

/// Maps a run's thread to a sandbox and keeps it running.
#[async_trait::async_trait]
pub trait SandboxBinder: Send + Sync + std::fmt::Debug {
    /// Return the thread's sandbox, ready for a tool call: running → as is,
    /// paused → resumed, gone or never created → created on the thread's
    /// volume.
    async fn ensure(&self, ctx: &ExecutorContext) -> SandboxResult<BoundSandbox>;
    /// Record the working directory a `Bash` call ended in.
    async fn set_cwd(&self, ctx: &ExecutorContext, cwd: &str) -> SandboxResult<()>;
}
