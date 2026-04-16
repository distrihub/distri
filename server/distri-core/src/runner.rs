use async_trait::async_trait;
use distri_types::RuntimeMode;

/// Trait for running agent execution in the background.
///
/// When `spawn()` is called, the implementation starts agent execution
/// and returns immediately. The caller monitors progress by subscribing
/// to the `AgentEventBroadcaster` for the given task_id.
///
/// Implementations:
/// - `InProcessRunner`: runs the agent loop via `tokio::spawn` (for distri-server)
/// - `SandboxLauncher`: spawns a browsr container with distri-cli (for distri-cloud)
#[async_trait]
pub trait BackgroundRunner: Send + Sync + 'static {
    /// Spawn agent execution in the background. Returns immediately.
    ///
    /// The caller should subscribe to the broadcaster for `task_id` to receive
    /// events and detect completion (RunFinished/RunError).
    async fn spawn(
        &self,
        task_id: String,
        agent_name: String,
        task: String,
        user_id: String,
        workspace_id: Option<String>,
        environment_id: Option<String>,
        thread_id: Option<String>,
    ) -> anyhow::Result<()>;

    /// The runtime that tasks dispatched to this runner will execute under.
    ///
    /// Used by the orchestrator when an agent declares a runtime constraint
    /// that doesn't match the current runtime — the orchestrator dispatches
    /// remote only when this matches the agent's required runtime.
    ///
    /// Default: `RuntimeMode::Cli`. `SandboxLauncher` runs `distri-cli` inside
    /// a browsr container, which executes in CLI runtime.
    fn provided_runtime(&self) -> RuntimeMode {
        RuntimeMode::Cli
    }
}
