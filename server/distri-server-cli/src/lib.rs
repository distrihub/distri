use anyhow::Result;
use distri_core::{
    agent::{AgentOrchestrator, PromptRegistry},
    AgentOrchestratorBuilder,
};
use distri_types::browser::BrowsrClientConfig;
use distri_types::configuration::{AgentConfig, DistriServerConfig};
pub mod workspace;
use std::{env, fs, path::Path, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::debug;

mod cli;
pub mod logging;

pub use cli::Cli;

/// Load distri.toml file and use its directory as home directory.
/// Returns (config, home_dir). Uses default configuration if no distri.toml is found.
pub fn load_distri_config(
    config_path: &Option<PathBuf>,
) -> Option<DistriServerConfig> {
    let toml_path = config_path.clone().or_else(|| {
        let default = std::env::current_dir().ok()?.join("distri.toml");
        default.exists().then_some(default)
    })?;

    let config_str = std::fs::read_to_string(&toml_path).ok()?;
    let config_str = replace_env_vars(&config_str);
    let config: DistriServerConfig = toml::from_str(&config_str).ok()?;

    debug!("Config loaded from {}: {:?}", toml_path.display(), config);
    Some(config)
}

/// Replace environment variables in config string ({{ENV_VAR}} format)
pub fn replace_env_vars(content: &str) -> String {
    let mut result = content.to_string();
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();
    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let env_var_name = cap.get(1).unwrap().as_str();
        if let Ok(env_value) = env::var(env_var_name) {
            result = result.replace(full_match, &env_value);
        }
    }
    result
}

/// Initialize the orchestrator for the OSS server.
pub async fn init_orchestrator(
    home_dir: &Path,
    workspace_path: &Path,
    workspace_config: Option<&DistriServerConfig>,
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

    // Resolve workspace config
    let mut resolved_config = workspace_config.cloned();
    if resolved_config.is_none() {
        let candidate = workspace_path.join("distri.toml");
        if candidate.exists() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                let content = replace_env_vars(&content);
                if let Ok(cfg) = toml::from_str::<DistriServerConfig>(&content) {
                    resolved_config = Some(cfg);
                }
            }
        }
    }

    let configuration_handle = Arc::new(RwLock::new(
        resolved_config
            .clone()
            .unwrap_or_else(DistriServerConfig::default),
    ));

    let merged_config = resolved_config.as_ref().or(workspace_config);

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
        .with_configuration(configuration_handle)
        .with_browser_config(BrowsrClientConfig::default())
        .with_stores(stores)
        .with_prompt_registry(prompt_registry)
        .with_store_config(store_config)
        .with_session_storage_path(workspace_path.join(".distri/session_storage"))
        .with_workspace_filesystem(workspace_fs)
        .build()
        .await?;

    let orchestrator = Arc::new(orchestrator);
    register_workspace_assets(&orchestrator, workspace_path, merged_config).await?;

    Ok(orchestrator)
}

async fn register_workspace_assets(
    orchestrator: &Arc<AgentOrchestrator>,
    workspace_path: &Path,
    workspace_config: Option<&DistriServerConfig>,
) -> Result<()> {
    let agents_dir = workspace_path.join("agents");
    if agents_dir.exists() {
        let workspace_package = workspace_config.and_then(|config| {
            let trimmed = config.name.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let local_agents = distri_core::agent::load_agents_from_dir(&agents_dir).await?;
        for mut definition in local_agents {
            definition.package_name = workspace_package.clone();
            orchestrator
                .stores
                .agent_store
                .register(AgentConfig::StandardAgent(definition))
                .await?;
        }
    }
    Ok(())
}
