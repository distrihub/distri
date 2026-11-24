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
        // The person running this server owns the machine: workflow `script` steps may run
        // a shell here (the cloud never allows it).
        .with_allow_host_shell(true);
    // Workflow `code` steps run with this machine's Deno (the person running the server owns it).
    match distri_core::code_runner::LocalDenoRunner::detect() {
        Some(runner) => builder = builder.with_code_runner(Arc::new(runner)),
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
