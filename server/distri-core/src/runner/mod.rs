use async_trait::async_trait;
use distri_types::RuntimeMode;

/// Trait for running agent execution in the background, outside this
/// process's own runtime.
///
/// When `spawn()` is called, the implementation starts agent execution
/// and returns immediately. The caller monitors progress by subscribing
/// to the `AgentEventBroadcaster` for the given task_id.
///
/// `distri-core` only defines this trait — it has no concrete
/// implementation and no knowledge of sandboxes/containers. The concrete
/// implementation (spawning a browsr container running `distri-cli`)
/// lives in `distri-cloud` (`cloud::runner::SandboxLauncher`), scoped to
/// the specific tool that needs it (e.g. a channel-only "run this in the
/// background" tool) rather than wired as an automatic fallback for every
/// runtime mismatch.
#[async_trait]
pub trait RemoteTaskRunner: Send + Sync + 'static {
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
    /// Default: `RuntimeMode::Cli` — a sandboxed runner typically runs
    /// `distri-cli` inside a container, which executes in CLI runtime.
    fn provided_runtime(&self) -> RuntimeMode {
        RuntimeMode::Cli
    }
}
