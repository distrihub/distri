use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use distri::run::{run_agent, RunOptions};
use distri::{AgentStreamClient, Distri};
use distri_core::broadcast::AgentEventBroadcaster;
use distri_core::runner::BackgroundRunner;
use distri_types::{AgentEvent, AgentEventType, DistriConfig, RuntimeMode};

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

    pub fn attach_broadcaster(&self, broadcaster: Arc<dyn AgentEventBroadcaster>) {
        if self.broadcaster.set(broadcaster).is_err() {
            tracing::warn!(
                "LocalProcessRemoteRunner::attach_broadcaster called more than once; keeping the original broadcaster"
            );
        }
    }

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

pub fn maybe_dev_mode_runner(
    provided_runtime: RuntimeMode,
) -> anyhow::Result<Option<Arc<LocalProcessRemoteRunner>>> {
    if is_dev_mode_enabled() {
        Ok(Some(Arc::new(LocalProcessRemoteRunner::from_env(
            provided_runtime,
        )?)))
    } else {
        Ok(None)
    }
}

fn is_dev_mode_enabled() -> bool {
    matches!(
        std::env::var("DEV_MODE")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
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
