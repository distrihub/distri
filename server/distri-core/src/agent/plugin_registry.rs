use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use distri_plugin_executor::plugin_trait::PluginLoadContext;
use distri_plugin_executor::{OrchestratorTrait, PluginExecutor, UnifiedPluginSystem};
use distri_plugins::PluginLoader;
use distri_types::configuration::PluginArtifact;
use distri_types::stores::{PluginCatalogStore, PluginMetadataRecord};
use distri_types::OrchestratorRef;
use object_store::ObjectStore;
use tokio::fs;
use tracing::{debug, error};

use crate::agent::parse_agent_markdown_content;
use crate::agent::plugin_storage::{build_object_store, ObjectStorePluginResolver};

/// Plugin loading options
#[derive(Debug, Clone)]
pub struct PluginOptions {
    /// Object store configuration backing plugin artifacts
    pub object_store: distri_types::configuration::ObjectStorageConfig,
    /// Whether to load compile-time default agents
    pub load_default_agents: bool,
    /// Optional filesystem path to scan for plugin directories. When omitted, the
    /// object store base path will be scanned (if it is filesystem-backed).
    pub filesystem_scan_path: Option<PathBuf>,
}

impl Default for PluginOptions {
    fn default() -> Self {
        Self {
            object_store: distri_types::configuration::ObjectStorageConfig::FileSystem {
                base_path: ".distri/plugins".to_string(),
            },
            load_default_agents: true,
            filesystem_scan_path: None,
        }
    }
}

pub struct PluginRegistry {
    metadata_store: Arc<dyn PluginCatalogStore>,
    object_store: Arc<RwLock<Arc<dyn ObjectStore>>>,
    pub plugin_system: Arc<UnifiedPluginSystem>,
    orchestrator_ref: Arc<OrchestratorRef>,
}

impl PluginRegistry {
    pub fn cleanup(&self) {
        self.plugin_system.cleanup();
    }

    pub async fn refresh_plugins_from_filesystem(
        &self,
        base_path: &Path,
        object_prefix_root: Option<&str>,
    ) -> Result<()> {
        self.sync_filesystem_plugins(base_path, object_prefix_root)
            .await
    }

    pub async fn register_workspace_module(&self, workspace_root: &Path) -> Result<()> {
        let loader = PluginLoader::new();
        let entrypoint = workspace_root.join("src/mod.ts");
        if !entrypoint.exists() {
            return Ok(());
        }

        let package_name = workspace_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace");
        let artifact = loader
            .load_plugin(package_name, workspace_root.to_path_buf())
            .await?;

        let record = PluginMetadataRecord {
            package_name: artifact.name.clone(),
            version: Some(artifact.configuration.version.clone()),
            object_prefix: ".".to_string(),
            entrypoint: artifact
                .configuration
                .entrypoints
                .as_ref()
                .map(|entry| entry.path.clone()),
            artifact,
            updated_at: Utc::now(),
        };

        self.register_plugin_artifact(record).await
    }

    pub fn new(metadata_store: Arc<dyn PluginCatalogStore>) -> Result<Self> {
        let default_store = build_object_store(&PluginOptions::default().object_store)?;
        let orchestrator_ref = Arc::new(OrchestratorRef::new());
        let plugin_system = Arc::new(UnifiedPluginSystem::new(orchestrator_ref.clone()));

        Ok(Self {
            metadata_store,
            object_store: Arc::new(RwLock::new(default_store)),
            plugin_system,
            orchestrator_ref,
        })
    }

    pub fn set_orchestrator(&self, orchestrator: Arc<dyn OrchestratorTrait>) {
        self.orchestrator_ref.set_orchestrator(orchestrator.clone());
        self.plugin_system.set_orchestrator(orchestrator);
    }

    pub async fn load_with_options(&self, options: PluginOptions) -> Result<()> {
        let store = build_object_store(&options.object_store)?;
        *self.object_store.write().unwrap() = store;

        if options.load_default_agents {
            self.load_default_agents().await?;
        }

        Ok(())
    }

    pub async fn register_plugin_artifact(&self, record: PluginMetadataRecord) -> Result<()> {
        self.metadata_store
            .upsert_plugin(&record)
            .await
            .context("Failed to persist plugin metadata")?;

        Ok(())
    }

    pub async fn list_plugin_records(&self) -> Result<Vec<PluginMetadataRecord>> {
        self.metadata_store
            .list_plugins()
            .await
            .context("Failed to load plugin metadata from store")
    }

    async fn load_plugin_artifact(&self, metadata: &PluginMetadataRecord) -> Result<()> {
        if metadata.entrypoint.is_none() {
            return Ok(());
        }

        let store = self.object_store.read().unwrap().clone();
        let resolver = Arc::new(ObjectStorePluginResolver::new(
            store,
            metadata.object_prefix.clone(),
        )?);

        let context = PluginLoadContext {
            package_name: metadata.package_name.clone(),
            entrypoint: metadata.entrypoint.clone(),
            manifest: metadata.artifact.configuration.clone(),
            resolver,
        };

        self.plugin_system
            .load_plugin(context)
            .await
            .with_context(|| format!("Failed to load plugin '{}'", metadata.package_name))
            .map(|_| ())
    }

    pub async fn ensure_plugin_loaded(&self, package_name: &str) -> Result<()> {
        let metadata = self
            .metadata_store
            .get_plugin(package_name)
            .await
            .with_context(|| format!("Failed to fetch plugin '{}'", package_name))?
            .ok_or_else(|| anyhow!("Plugin '{}' is not registered", package_name))?;

        self.load_plugin_artifact(&metadata).await
    }

    pub async fn get_plugin_tools(
        &self,
    ) -> Result<HashMap<String, Vec<Arc<dyn crate::tools::Tool>>>> {
        let mut result = HashMap::new();

        for metadata in self.list_plugin_records().await? {
            if metadata.artifact.tools.is_empty() {
                continue;
            }

            let mut package_tools: Vec<Arc<dyn crate::tools::Tool>> = Vec::new();
            for tool_def in &metadata.artifact.tools {
                let plugin_tool = crate::tools::PluginTool::new(
                    tool_def.name.clone(),
                    tool_def.description.clone(),
                    tool_def.parameters.clone(),
                    metadata.package_name.clone(),
                    PathBuf::from(metadata.object_prefix.clone()),
                    tool_def.auth.clone(),
                );

                package_tools.push(Arc::new(plugin_tool) as Arc<dyn crate::tools::Tool>);
            }

            if !package_tools.is_empty() {
                result.insert(metadata.package_name.clone(), package_tools);
            }
        }

        Ok(result)
    }

    pub async fn get_all_tools(&self) -> Result<Vec<Arc<dyn crate::tools::Tool>>> {
        let mut all_tools = Vec::new();
        for (_package, tools) in self.get_plugin_tools().await? {
            all_tools.extend(tools);
        }
        Ok(all_tools)
    }

    pub async fn load_default_agents(&self) -> Result<()> {
        debug!("Loading default agents from compile-time included files");

        let package_name = "distri".to_string();

        let default_agents = vec![
            ("distri", include_str!("../../../agents/distri.md")),
            ("scripter", include_str!("../../../agents/scripter.md")),
            ("search", include_str!("../../../agents/search.md")),
            ("browser_agent", include_str!("../../../agents/browser.md")),
            (
                "deepresearch",
                include_str!("../../../agents/deepresearch.md"),
            ),
            (
                "agent_designer",
                include_str!("../../../agents/agent_designer.md"),
            ),
            ("web_agent", include_str!("../../../agents/web_agent.md")),
        ];

        let mut loaded_agents = Vec::new();

        for (agent_name, content) in default_agents {
            debug!("Loading default agent: {}", agent_name);
            match parse_agent_markdown_content(content).await {
                Ok(mut agent_def) => {
                    agent_def.package_name = Some(package_name.clone());
                    debug!("Successfully loaded agent: {}", agent_def.name);
                    loaded_agents.push(agent_def);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse agent {}: {}", agent_name, e);
                }
            }
        }

        if loaded_agents.is_empty() {
            return Ok(());
        }

        let dap_agents = loaded_agents
            .into_iter()
            .map(
                |agent_def| distri_types::configuration::PluginAgentDefinition {
                    name: agent_def.name.clone(),
                    package_name: package_name.clone(),
                    description: agent_def.description.clone(),
                    file_path: PathBuf::from("<default>"),
                    agent_config: distri_types::configuration::AgentConfig::StandardAgent(
                        agent_def,
                    ),
                },
            )
            .collect();

        let artifact = PluginArtifact {
            name: package_name.clone(),
            path: PathBuf::from("<default>"),
            configuration: distri_types::configuration::DistriServerConfig {
                name: package_name.clone(),
                version: "1.0.0".to_string(),
                description: Some("Default agents built into Distri".to_string()),
                ..Default::default()
            },
            tools: Vec::new(),
            workflows: Vec::new(),
            agents: dap_agents,
        };

        let record = PluginMetadataRecord {
            package_name: package_name.clone(),
            version: Some("1.0.0".to_string()),
            object_prefix: "<default>".to_string(),
            entrypoint: None,
            artifact,
            updated_at: Utc::now(),
        };

        self.register_plugin_artifact(record).await
    }

    pub fn plugin_system(&self) -> Arc<UnifiedPluginSystem> {
        self.plugin_system.clone()
    }

    pub async fn sync_filesystem_plugins(
        &self,
        base_path: &Path,
        object_prefix_root: Option<&str>,
    ) -> Result<()> {
        if !base_path.exists() {
            debug!(
                "Filesystem plugin directory does not exist, skipping sync: {:?}",
                base_path
            );
            return Ok(());
        }

        let mut dir = match fs::read_dir(base_path).await {
            Ok(dir) => dir,
            Err(err) => {
                return Err(anyhow!(
                    "Failed to read plugin directory {}: {}",
                    base_path.display(),
                    err
                ));
            }
        };

        let loader = PluginLoader::new();

        while let Some(entry) = dir
            .next_entry()
            .await
            .context("Failed to iterate plugin directory")?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .context("Failed to read plugin entry type")?;

            if !file_type.is_dir() {
                continue;
            }

            debug!("Evaluating plugin candidate at {}", path.display());

            let candidate_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            match loader.load_plugin(candidate_name, path.clone()).await {
                Ok(artifact) => {
                    let entrypoint =
                        artifact
                            .configuration
                            .entrypoints
                            .as_ref()
                            .and_then(|entry| match entry {
                                distri_types::configuration::EntryPoints { path } => {
                                    Some(path.clone())
                                }
                            });

                    let relative_prefix = path
                        .strip_prefix(base_path)
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|_| path.clone());
                    let mut object_prefix = relative_prefix.to_string_lossy().replace("\\", "/");
                    if let Some(root) = object_prefix_root {
                        if !root.is_empty() {
                            object_prefix =
                                format!("{}/{}", root.trim_end_matches('/'), object_prefix);
                        }
                    }

                    let record = PluginMetadataRecord {
                        package_name: artifact.name.clone(),
                        version: Some(artifact.configuration.version.clone()),
                        object_prefix,
                        entrypoint,
                        artifact,
                        updated_at: Utc::now(),
                    };

                    // Remove stale metadata if the on-disk directory name changed
                    if record.package_name != candidate_name {
                        if let Err(err) = self.metadata_store.remove_plugin(candidate_name).await {
                            debug!(
                                "Failed to remove stale plugin metadata for {}: {}",
                                candidate_name, err
                            );
                        }
                    }

                    if let Err(err) = self.metadata_store.upsert_plugin(&record).await {
                        error!(
                            "Failed to upsert plugin metadata for {}: {}",
                            record.package_name, err
                        );
                    } else {
                        debug!(
                            "Registered plugin {} with prefix {}",
                            record.package_name, record.object_prefix
                        );
                    }
                }
                Err(err) => {
                    error!("Failed to load plugin at {}: {}", path.display(), err);
                }
            }
        }

        Ok(())
    }
}
