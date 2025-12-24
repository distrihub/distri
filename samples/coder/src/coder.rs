use anyhow::{Context, Result};
use distri::{
    AgentOrchestrator, AgentOrchestratorBuilder,
    agent::{PluginRegistry, PromptRegistry, parse_agent_markdown_content},
};

use crate::tools::ExecuteCommandTool;
use distri_filesystem::FileSystem;
use distri_types::configuration::ObjectStorageConfig;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Initialize DAP-based orchestrator using distri.toml home directory or .distri folder
pub async fn init_coder(
    home_dir: &Path,
    db_path: PathBuf,
    code_home: PathBuf,
) -> Result<std::sync::Arc<distri::agent::AgentOrchestrator>> {
    use distri_types::configuration::StoreConfig;

    // Configure stores for CLI - use persistent session stores for history
    let mut store_config = StoreConfig::default();
    store_config.session.ephemeral = false; // CLI needs persistent history

    let db_url = db_path.to_string_lossy().to_string();

    if let Some(ref mut metadata_db) = store_config.metadata.db_config {
        metadata_db.database_url = db_url.clone();
    } else {
        store_config.metadata.db_config = Some(distri_types::configuration::DbConnectionConfig {
            database_url: db_url.clone(),
            ..Default::default()
        });
    }

    store_config.session.db_config = Some(distri_types::configuration::DbConnectionConfig {
        database_url: db_url,
        ..Default::default()
    });

    let stores = distri::initialize_stores(&store_config).await?;

    let plugin_registry = PluginRegistry::new(stores.plugin_store.clone())?;

    // Initialize prompt registry with defaults and auto-discovery for CLI
    let prompt_registry = Arc::new(PromptRegistry::with_defaults().await?);

    // Auto-discover prompt templates from home directory (CLI-specific behavior)
    let prompt_templates_path = home_dir.join("prompt_templates");
    if prompt_templates_path.exists() {
        tracing::debug!(
            "Auto-registering prompt templates from: {}",
            prompt_templates_path.display()
        );
        prompt_registry
            .register_templates_from_directory(&prompt_templates_path)
            .await?;

        // Also register partials from the partials subdirectory
        let partials_path = prompt_templates_path.join("partials");
        if partials_path.exists() {
            prompt_registry
                .register_partials_from_directory(&partials_path)
                .await?;
        }
    }

    let file_system = init_fs(home_dir).await?;
    let builder = AgentOrchestratorBuilder::default();
    let orchestrator = builder
        .with_stores(stores)
        .with_workspace_file_system(Arc::new(file_system))
        .with_plugin_registry(Arc::new(plugin_registry))
        .with_prompt_registry(prompt_registry)
        .with_store_config(store_config)
        .build()
        .await?;

    let orchestrator = Arc::new(orchestrator);
    register_coder_agent(&orchestrator, &code_home).await?;
    Ok(orchestrator)
}

pub async fn register_coder_agent(
    executor: &Arc<AgentOrchestrator>,
    code_home: &Path,
) -> Result<()> {
    executor
        .stores
        .agent_store
        .clear()
        .await
        .context("failed to clear agent store before registering coder agent")?;
    let mut definition = parse_agent_markdown_content(include_str!("../agents/coder.md"))
        .await
        .map_err(|e| anyhow::anyhow!("failed to load coder agent definition: {}", e))?;

    // Allow the model to be overridden at runtime without editing the markdown file.
    if let Ok(model_override) = std::env::var("DISTRI_CODER_MODEL") {
        definition.model_settings.model = model_override;
    }

    let agent_name = definition.name.clone();
    executor.register_agent_definition(definition).await?;
    executor
        .register_tool(&agent_name, Arc::new(ExecuteCommandTool::new(code_home)))
        .await;
    Ok(())
}

pub async fn init_fs(home_dir: &Path) -> Result<FileSystem> {
    let object_store_config = ObjectStorageConfig::FileSystem {
        base_path: home_dir.to_string_lossy().to_string(),
    };

    let fs_config = distri_filesystem::FileSystemConfig {
        object_store: object_store_config,
        root_prefix: None,
    };

    let fs = distri_filesystem::create_file_system(fs_config).await?;
    Ok(fs)
}
