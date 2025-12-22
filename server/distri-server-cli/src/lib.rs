use anyhow::Result;
use distri_core::{
    agent::{AgentOrchestrator, PluginOptions, PluginRegistry, PromptRegistry},
    types::{McpServerMetadata, ServerTrait, TransportType},
    AgentOrchestratorBuilder,
};
use distri_types::browser::DistriBrowserConfig;
use distri_types::configuration::{AgentConfig, DistriConfiguration, ObjectStorageConfig};
use distri_types::ServerMetadataWrapper;
pub mod workspace;
use crate::tool_renderers::ToolRendererRegistry;
use std::{collections::HashMap, env, fs, path::Path, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::debug;

mod cli;
pub mod handlers;
pub mod logging;
mod shared_state;
pub use shared_state::SharedState;
pub mod multi_agent_cli;
pub mod run;
pub mod slash_commands;
pub mod tool_renderers;
pub mod workflow_dag;
// use run::{background, chat}; // Unused imports

pub use cli::{
    AuthCommand, AuthSecretsCommand, Cli, Commands, EmbeddedCli, EmbeddedCommands,
    ScratchpadCommands,
};

/// Load distri.toml file and use its directory as home directory
/// Uses default configuration if no distri.toml is found
pub fn load_distri_config(
    config_path: &Option<PathBuf>,
) -> Result<(Option<DistriConfiguration>, PathBuf)> {
    let current_dir = std::env::current_dir()?;

    let (distri_config_path, home_dir) = match config_path {
        Some(path) => {
            if path.exists() {
                let home = path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf();
                (Some(path.clone()), home)
            } else {
                return Err(anyhow::anyhow!(
                    "distri.toml file not found at: {}",
                    path.display()
                ));
            }
        }
        None => {
            // Check for distri.toml in current directory
            let default_path = current_dir.join("distri.toml");
            if default_path.exists() {
                let home = default_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf();
                (Some(default_path), home)
            } else {
                // No configuration file found, use default settings
                debug!("No distri.toml found, using default configuration");
                (None, current_dir.clone())
            }
        }
    };

    // Load distri.toml if it exists, otherwise return None
    let config = if let Some(toml_path) = distri_config_path {
        let config_str = std::fs::read_to_string(&toml_path)?;
        let config_str = replace_env_vars(&config_str);
        let config: DistriConfiguration = toml::from_str(&config_str)?;

        debug!(
            "DAP config loaded from {}: {:?}",
            toml_path.display(),
            config
        );
        Some(config)
    } else {
        debug!("Using .distri folder configuration without distri.toml");
        None
    };

    debug!("Using home directory: {}", home_dir.display());

    Ok((config, home_dir))
}

/// Replace environment variables in config string ({{ENV_VAR}} format)
pub fn replace_env_vars(content: &str) -> String {
    let mut result = content.to_string();

    // Find all patterns matching {{ENV_VAR}}
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

/// Initialize DAP-based orchestrator using distri.toml home directory or .distri folder
pub async fn init_orchestrator(
    home_dir: &Path,
    workspace_path: &Path,
    workspace_config: Option<&DistriConfiguration>,
    disable_plugins: bool,
    headless_browser: bool,
) -> Result<std::sync::Arc<distri_core::agent::AgentOrchestrator>> {
    init_orchestrator_with_configuration(
        home_dir,
        workspace_path,
        workspace_config,
        disable_plugins,
        headless_browser,
        None,
    )
    .await
}

/// Initialize orchestrator with a shared configuration handle that can be updated at runtime.
pub async fn init_orchestrator_with_configuration(
    home_dir: &Path,
    workspace_path: &Path,
    workspace_config: Option<&DistriConfiguration>,
    disable_plugins: bool,
    headless_browser: bool,
    configuration: Option<Arc<RwLock<DistriConfiguration>>>,
) -> Result<std::sync::Arc<distri_core::agent::AgentOrchestrator>> {
    use distri_types::configuration::StoreConfig;

    // Create store config that uses file-based stores with ~/.distri directory
    let distri_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
        .join(".distri");

    // Ensure ~/.distri directory exists
    std::fs::create_dir_all(&distri_dir)?;

    // Configure stores for CLI - use persistent session stores for history
    let mut store_config = StoreConfig::default();
    store_config.session.ephemeral = false; // CLI needs persistent history

    let stores = distri_core::initialize_stores(&store_config).await?;

    let plugin_registry = PluginRegistry::new(stores.plugin_store.clone())?;

    if !disable_plugins {
        let mut plugin_options = PluginOptions::default();
        plugin_options.object_store = ObjectStorageConfig::FileSystem {
            base_path: workspace_path.to_string_lossy().to_string(),
        };

        plugin_registry.load_with_options(plugin_options).await?;
    }

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

    let mut resolved_config_owned = workspace_config.cloned();
    if resolved_config_owned.is_none() {
        let candidate = workspace_path.join("distri.toml");
        if candidate.exists() {
            match fs::read_to_string(&candidate) {
                Ok(content) => {
                    let content = replace_env_vars(&content);
                    match toml::from_str::<DistriConfiguration>(&content) {
                        Ok(cfg) => resolved_config_owned = Some(cfg),
                        Err(error) => tracing::warn!(
                            "Failed to parse distri.toml at {}: {}",
                            candidate.display(),
                            error
                        ),
                    }
                }
                Err(error) => tracing::warn!(
                    "Failed to read distri.toml at {}: {}",
                    candidate.display(),
                    error
                ),
            }
        }
    }

    let configuration_handle = if let Some(handle) = configuration {
        if resolved_config_owned.is_none() {
            resolved_config_owned = Some(handle.read().await.clone());
        }
        handle
    } else {
        Arc::new(RwLock::new(
            resolved_config_owned
                .clone()
                .unwrap_or_else(DistriConfiguration::default),
        ))
    };

    let merged_config = resolved_config_owned.as_ref().or(workspace_config);

    let mut builder = AgentOrchestratorBuilder::default().with_configuration(configuration_handle);
    if let Some(config) = merged_config {
        if let Some(model_settings) = config.model_settings.clone() {
            builder = builder.with_default_model_settings(model_settings);
        }
        if let Some(analysis_settings) = config.analysis_model_settings.clone() {
            builder = builder.with_default_analysis_model_settings(analysis_settings);
        }
    }
    builder = builder.with_browser_config(DistriBrowserConfig {
        headless: Some(headless_browser),
        ..Default::default()
    });
    let orchestrator = builder
        .with_stores(stores)
        .with_plugin_registry(Arc::new(plugin_registry))
        .with_prompt_registry(prompt_registry)
        .with_store_config(store_config)
        .with_workspace_path(workspace_path.to_path_buf())
        .build()
        .await?;

    let orchestrator = Arc::new(orchestrator);
    register_workspace_assets(&orchestrator, workspace_path, merged_config).await?;

    if !disable_plugins {
        for mcp_server in custom_mcp_servers() {
            orchestrator
                .register_mcp_server(mcp_server.0, mcp_server.1)
                .await;
        }
    }
    Ok(orchestrator)
}

async fn register_workspace_assets(
    orchestrator: &Arc<AgentOrchestrator>,
    workspace_path: &Path,
    workspace_config: Option<&DistriConfiguration>,
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

pub async fn build_workspace(
    orchestrator: &Arc<AgentOrchestrator>,
    workspace_path: &Path,
) -> Result<()> {
    let plugins_dir = workspace_path.join("plugins");
    orchestrator
        .plugin_registry
        .refresh_plugins_from_filesystem(&plugins_dir, Some("plugins"))
        .await?;
    orchestrator
        .plugin_registry
        .register_workspace_module(workspace_path)
        .await?;
    tracing::info!(
        "Workspace build complete (plugins from {}, workspace src)",
        plugins_dir.display()
    );
    Ok(())
}

/// Run CLI with the given configuration (DAP-aware)
pub async fn run_agent_cli(
    executor: Arc<AgentOrchestrator>,
    agent_name: &str,
    input: Option<&str>,
    user_id: Option<&str>,
    verbose: bool,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
    headless_browser: bool,
) -> Result<()> {
    debug!("Running agent: {:?}", agent_name);

    // Use DAP-aware run function (config not needed for DAP mode)
    if let Some(input_str) = input {
        let task_msg = distri_types::Message::user(input_str.to_string(), None);
        run::background::run(
            agent_name,
            executor,
            task_msg,
            verbose,
            user_id,
            tool_renderers,
        )
        .await?;
    } else {
        run::chat::run(
            agent_name,
            executor,
            verbose,
            tool_renderers,
            headless_browser,
        )
        .await?;
    }
    Ok(())
}

/// Get custom MCP servers (spider, search)
pub fn custom_mcp_servers() -> HashMap<String, ServerMetadataWrapper> {
    let mut servers = HashMap::new();

    // Add Spider scraping server
    servers.insert(
        "spider".to_string(),
        ServerMetadataWrapper {
            server_metadata: McpServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                auth_type: None,
            },
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_crawl::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    // Add Tavily search server
    servers.insert(
        "search".to_string(),
        ServerMetadataWrapper {
            server_metadata: McpServerMetadata {
                auth_session_key: None,
                mcp_transport: TransportType::InMemory,
                auth_type: None,
            },
            builder: Some(Arc::new(|_, transport| {
                let server = mcp_tavily::build(transport)?;
                Ok(Box::new(server) as Box<dyn ServerTrait>)
            })),
        },
    );

    servers
}
