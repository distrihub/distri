use anyhow::Result;
use distri_core::{
    agent::{AgentOrchestrator, PromptRegistry},
    AgentOrchestratorBuilder,
};
use distri_types::browser::BrowsrClientConfig;
use distri_types::configuration::AgentConfig;
pub mod workspace;
use std::{path::Path, sync::Arc};

mod cli;
pub mod logging;

pub use cli::Cli;

/// Initialize the orchestrator for the OSS server.
pub async fn init_orchestrator(
    home_dir: &Path,
    workspace_path: &Path,
) -> Result<Arc<AgentOrchestrator>> {
    use distri_types::configuration::StoreConfig;

    let distri_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join(".distri");
    std::fs::create_dir_all(&distri_dir)?;

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

    let orchestrator = AgentOrchestratorBuilder::default()
        .with_browser_config(BrowsrClientConfig::default())
        .with_stores(stores)
        .with_prompt_registry(prompt_registry)
        .with_store_config(store_config)
        .with_session_storage_path(workspace_path.join(".distri/session_storage"))
        .with_workspace_filesystem(workspace_fs)
        .build()
        .await?;

    let orchestrator = Arc::new(orchestrator);
    register_workspace_agents(&orchestrator, workspace_path).await?;

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
