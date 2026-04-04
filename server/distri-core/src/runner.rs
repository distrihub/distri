use async_trait::async_trait;

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
    ) -> anyhow::Result<()>;
}
