//! `LocalProcessRemoteRunner` — `BackgroundRunner` implementation that runs
//! `--remote` calls via the `distri` client library in-process, going
//! through the SAME code path the distri CLI uses (`distri::run::run_agent`).
//! No docker container, no subprocess — but the full HTTP/SSE + A2A
//! contract is exercised because `run_agent` uses
//! `AgentStreamClient::stream_agent` which POSTs to our own server.
//!
//! Lives in `distri-core` (rather than cloud) so OSS distri-server can
//! also wire it up as a fallback runner when no sandboxed runtime is
//! available. Cloud opts in via `LOCAL_SANDBOX_MODE=true`.
//!
//! Strictly lower-fidelity than cloud's `SandboxLauncher` (no sandbox
//! isolation, no browsr container, no full tracing parity).
//!
//! ## Construction + broadcaster attach pattern
//!
//! The runner doesn't strictly need the orchestrator's broadcaster on the
//! happy path (events flow through the server's normal HTTP handler when
//! the loopback request lands). It only needs it to synthesize a terminal
//! `RunError` if `run_agent` itself fails (before the server ever saw a
//! request). Because the broadcaster lives on the orchestrator and the
//! orchestrator's builder consumes the runner, we use a post-build
//! `attach_broadcaster` setter to break the cycle:
//!
//! ```ignore
//! let runner = Arc::new(LocalProcessRemoteRunner::from_env(RuntimeMode::Cli)?);
//! let orchestrator = AgentOrchestratorBuilder::default()
//!     .with_background_runner(runner.clone())
//!     .build().await?;
//! runner.attach_broadcaster(orchestrator.runtime.broadcaster_arc());
//! ```
//!
//! If the broadcaster was never attached, a `run_agent` failure simply
//! logs and returns — no synthesized terminal event. That's acceptable
//! because the server-side path would normally handle the request and
//! emit its own terminal.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use distri::run::{run_agent, RunOptions};
use distri::{AgentStreamClient, Distri};
use distri_types::{AgentEvent, AgentEventType, DistriConfig, RuntimeMode};

use crate::broadcast::AgentEventBroadcaster;
use crate::runner::BackgroundRunner;

/// `BackgroundRunner` that drives `--remote` calls through the `distri`
/// client library against the same server process. DEV_MODE=true only.
pub struct LocalProcessRemoteRunner {
    platform_client: Distri,
    stream_client: AgentStreamClient,
    broadcaster: OnceLock<Arc<dyn AgentEventBroadcaster>>,
    provided_runtime: RuntimeMode,
}

impl LocalProcessRemoteRunner {
    pub fn new(
        platform_client: Distri,
        stream_client: AgentStreamClient,
        provided_runtime: RuntimeMode,
    ) -> Self {
        Self {
            platform_client,
            stream_client,
            broadcaster: OnceLock::new(),
            provided_runtime,
        }
    }

    /// Attach the orchestrator's broadcaster post-build. See module-level
    /// doc for the full pattern. Idempotent on the first call — a second
    /// call is ignored with a `tracing::warn!`.
    pub fn attach_broadcaster(&self, broadcaster: Arc<dyn AgentEventBroadcaster>) {
        if self.broadcaster.set(broadcaster).is_err() {
            tracing::warn!(
                "LocalProcessRemoteRunner::attach_broadcaster called more than once; keeping the original broadcaster"
            );
        }
    }

    /// Build from env. Reads:
    /// - `DISTRI_CLOUD_URL` or `DISTRI_BASE_URL` → base URL (required).
    /// - `DISTRI_API_KEY` or `SYSTEM_API_KEY` → api key (optional but
    ///   strongly recommended — without it the loopback call will hit
    ///   the server as unauthenticated).
    pub fn from_env(provided_runtime: RuntimeMode) -> anyhow::Result<Self> {
        let base_url = std::env::var("DISTRI_CLOUD_URL")
            .or_else(|_| std::env::var("DISTRI_BASE_URL"))
            .map_err(|_| {
                anyhow::anyhow!(
                    "LocalProcessRemoteRunner: neither DISTRI_CLOUD_URL nor DISTRI_BASE_URL is set"
                )
            })?;

        let mut config = DistriConfig::new(base_url);
        if let Some(key) = std::env::var("DISTRI_API_KEY")
            .ok()
            .or_else(|| std::env::var("SYSTEM_API_KEY").ok())
        {
            config = config.with_api_key(key);
        }

        let platform_client = Distri::from_config(config.clone());
        let stream_client = AgentStreamClient::from_config(config);

        Ok(Self::new(platform_client, stream_client, provided_runtime))
    }
}

#[async_trait]
impl BackgroundRunner for LocalProcessRemoteRunner {
    async fn spawn(
        &self,
        task_id: String,
        agent_name: String,
        task: String,
        user_id: String,
        workspace_id: Option<String>,
        _environment_id: Option<String>,
        thread_id: Option<String>,
    ) -> anyhow::Result<()> {
        let platform = self.platform_client.clone();
        let stream = self.stream_client.clone();
        let broadcaster = self.broadcaster.get().cloned();
        let task_id_clone = task_id.clone();
        let agent_clone = agent_name.clone();

        // Parse workspace_id for the task-local context installation —
        // TenantAgentStore / TenantSecretStore look it up via
        // with_user_and_workspace. Without this wrapper the spawned task
        // runs with current_user=None and every store lookup fails.
        let ws_uuid = workspace_id
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        tokio::spawn(distri_auth::context::with_user_and_workspace(
            user_id,
            ws_uuid,
            async move {
                let opts = RunOptions {
                    agent: Some(agent_clone.clone()),
                    task,
                    task_id: Some(task_id_clone.clone()),
                    thread_id,
                    remote: true,
                    model: None,
                    env_vars: None,
                    // Server-side already has connection info; no need to
                    // ship it back up the wire.
                    skip_connections_context: true,
                };
                if let Err(e) = run_agent(&platform, &stream, opts, |_item| async {}).await {
                    tracing::error!(
                        task_id = %task_id_clone,
                        agent = %agent_clone,
                        error = ?e,
                        "LocalProcessRemoteRunner: run_agent failed"
                    );
                    if let Some(bc) = broadcaster {
                        // Synthesize a terminal so any caller subscribed
                        // to this task_id doesn't hang on drain.
                        let _ = bc
                            .publish(
                                &task_id_clone,
                                AgentEvent {
                                    timestamp: chrono::Utc::now(),
                                    thread_id: String::new(),
                                    run_id: String::new(),
                                    event: AgentEventType::RunError {
                                        message: format!("run_agent: {}", e),
                                        code: Some("LOCAL_RUN_FAILED".to_string()),
                                        usage: None,
                                    },
                                    task_id: task_id_clone.clone(),
                                    parent_task_id: None,
                                    agent_id: agent_clone.clone(),
                                    user_id: None,
                                    identifier_id: None,
                                    workspace_id: None,
                                    channel_id: None,
                                },
                            )
                            .await;
                    }
                }
            },
        ));
        Ok(())
    }

    fn provided_runtime(&self) -> RuntimeMode {
        self.provided_runtime.clone()
    }
}
