use anyhow::Result;
use distri_core::{
    agent::{AgentOrchestrator, PromptRegistry},
    AgentOrchestratorBuilder,
};
use distri_types::configuration::AgentConfig;
pub mod workspace;
use std::{path::Path, sync::Arc};

mod cli;
pub mod distri_yaml;
pub mod logging;
mod seed;

pub use cli::Cli;

/// Durable runs are on when `DISTRI_DURABLE_RUNS` is `1`/`true`/`on`.
/// Whether workflow `script` steps may run a shell on this machine: only when the person
/// running the server sets `DISTRI_ALLOW_HOST_SHELL=1` (or true/on/yes). Off by default, so an
/// agent or workflow definition alone can never run commands on the host.
pub fn host_shell_allowed() -> bool {
    host_shell_allowed_from(std::env::var("DISTRI_ALLOW_HOST_SHELL").ok().as_deref())
}

fn host_shell_allowed_from(value: Option<&str>) -> bool {
    value
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod host_shell_tests {
    #[test]
    fn host_shell_is_opt_in() {
        use super::host_shell_allowed_from as f;
        assert!(!f(None));
        assert!(!f(Some("")));
        assert!(!f(Some("0")));
        assert!(!f(Some("no")));
        assert!(f(Some("1")));
        assert!(f(Some("TRUE")));
    }
}

pub fn durable_runs_enabled() -> bool {
    std::env::var("DISTRI_DURABLE_RUNS")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

/// Where a workflow `code` step's script reaches this server's script SDK: `DISTRI_SDK_URL`, else
/// the address `main` sets from the bound host and port (`DISTRI_SDK_DEFAULT_URL`). `off` turns
/// the SDK off.
pub fn script_sdk_url() -> Option<String> {
    let url = std::env::var("DISTRI_SDK_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| SDK_DEFAULT_URL.get().cloned())?;
    (!url.trim().eq_ignore_ascii_case("off")).then(|| url.trim().to_string())
}

static SDK_DEFAULT_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The SDK URL for a server bound to `host:port` (a wildcard host is reached on loopback).
pub fn set_default_script_sdk_url(host: &str, port: u16) {
    let host = match host {
        "0.0.0.0" | "::" | "[::]" | "" => "127.0.0.1",
        h => h,
    };
    let _ = SDK_DEFAULT_URL.set(format!("http://{host}:{port}/v1/workflows/sdk"));
}

/// Initialize the orchestrator for the OSS server.
pub async fn init_orchestrator(
    home_dir: &Path,
    workspace_path: &Path,
    distri_config: &distri_yaml::DistriYamlConfig,
) -> Result<Arc<AgentOrchestrator>> {
    use distri_types::configuration::StoreConfig;

    let distri_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join(".distri");
    std::fs::create_dir_all(&distri_dir)?;

    // `distri.yaml` — the OSS declarative seed config. Provider/model
    // extensions (from distri.yaml, a `providers/` directory, or
    // DISTRI_MODEL_CATALOG) must be registered before the server serves the
    // catalog, so this happens up front; the default-model and agent seeds
    // are applied after the orchestrator is built.
    distri_yaml::register_extensions(workspace_path, Some(distri_config));

    let mut store_config = StoreConfig::default();
    store_config.session.ephemeral = false;

    let stores = distri_core::initialize_stores(&store_config).await?;

    let prompt_registry = Arc::new(PromptRegistry::with_defaults().await?);

    // Auto-discover prompt templates
    let prompt_templates_path = home_dir.join("prompt_templates");
    if prompt_templates_path.exists() {
        prompt_registry
            .register_templates_from_directory(&prompt_templates_path)
            .await?;
        let partials_path = prompt_templates_path.join("partials");
        if partials_path.exists() {
            prompt_registry
                .register_partials_from_directory(&partials_path)
                .await?;
        }
    }

    // Create workspace filesystem for file routes (not for agent tools)
    let workspace_fs = {
        let fs_config = distri_filesystem::FileSystemConfig {
            object_store: distri_types::configuration::ObjectStorageConfig::FileSystem {
                base_path: workspace_path.to_string_lossy().to_string(),
            },
            root_prefix: None,
        };
        Arc::new(distri_filesystem::create_file_system(fs_config).await?)
    };

    let durable_store_config = store_config.clone();
    let mut builder = AgentOrchestratorBuilder::default()
        .with_stores(stores)
        .with_prompt_registry(prompt_registry)
        .with_store_config(store_config)
        .with_session_storage_path(workspace_path.join(".distri/session_storage"))
        .with_workspace_filesystem(workspace_fs)
        // Workflow `script` steps run a shell here only when the person running the server
        // opts in with DISTRI_ALLOW_HOST_SHELL=1 (the cloud never allows it).
        .with_allow_host_shell(host_shell_allowed());
    // Workflow `code` steps run with this machine's Deno (the person running the server owns it).
    match distri_core::code_runner::LocalDenoRunner::detect() {
        Some(runner) => {
            builder = builder.with_code_runner(Arc::new(runner));
            // The script SDK (`@distri/runtime`): scripts call back to this server.
            if let Some(url) = script_sdk_url() {
                builder = builder.with_script_sdk_url(url);
            }
        }
        None => tracing::info!("deno not found: workflow code steps are unavailable"),
    }
    if durable_runs_enabled() {
        // Durable runs: the queue and pending calls live in the same sqlite
        // database as the tasks; the worker runs in this process.
        let stores = distri_core::initialize_durable_stores(&durable_store_config).await?;
        builder = builder.with_durable_runtime(distri_core::durable::DurableRuntime::new(
            stores.run_queue,
            stores.pending_calls,
            Arc::new(distri_core::InProcessWakeNotifier::new()),
            Arc::new(distri_core::durable::NoopResumeHook),
        ));
        tracing::info!("durable runs enabled (DISTRI_DURABLE_RUNS)");
    }
    let orchestrator = builder.build().await?;

    let orchestrator = Arc::new(orchestrator);
    orchestrator.start_durable_worker(true);
    seed::seed_bundled_defaults(orchestrator.as_ref()).await?;
    register_workspace_agents(&orchestrator, workspace_path).await?;

    distri_yaml::apply_runtime_seeds(distri_config, orchestrator.as_ref(), workspace_path).await?;

    Ok(orchestrator)
}

async fn register_workspace_agents(
    orchestrator: &Arc<AgentOrchestrator>,
    workspace_path: &Path,
) -> Result<()> {
    let agents_dir = workspace_path.join("agents");
    if agents_dir.exists() {
        let local_agents = distri_core::agent::load_agents_from_dir(&agents_dir).await?;
        for definition in local_agents {
            orchestrator
                .stores
                .agent_store
                .register(AgentConfig::StandardAgent(definition))
                .await?;
        }
    }
    Ok(())
}
