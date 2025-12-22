use anyhow::{anyhow, Context, Result};

use distri_plugin_executor::plugin_trait::{PluginExecutor, PluginFileResolver, PluginLoadContext};
use distri_types::configuration::EntryPoints;
use distri_types::{
    configuration::{
        PluginAgentDefinition, PluginArtifact, PluginToolDefinition, PluginWorkflowDefinition,
    },
    DistriServerConfig,
};
use distri_types::{MockOrchestrator, StandardDefinition};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tracing::debug;

struct LocalPluginResolver {
    root: PathBuf,
}

impl LocalPluginResolver {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl PluginFileResolver for LocalPluginResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let sanitized = path.trim_start_matches('/');
        let full_path = self.root.join(sanitized);
        std::fs::read(&full_path)
            .with_context(|| format!("Failed to read plugin file at {}", full_path.display()))
    }
}

/// DAP Package Loader - handles all loading logic
pub struct PluginLoader {}

impl PluginLoader {
    pub fn new() -> Self {
        Self {}
    }

    /// Load a specific distri package
    pub async fn load_plugin(
        &self,
        package_name: &str,
        package_path: PathBuf,
    ) -> Result<PluginArtifact> {
        debug!(
            "Loading distri package: {} from {:?}",
            package_name, package_path
        );

        // Load manifest
        let manifest_path = package_path.join("distri.toml");
        let configuration = if manifest_path.exists() {
            DistriServerConfig::load_from_path(&manifest_path).await?
        } else {
            let default_entry = if package_path.join("src/mod.ts").exists() {
                "src/mod.ts".to_string()
            } else {
                "mod.ts".to_string()
            };
            DistriServerConfig {
                name: package_name.to_string(),
                entrypoints: Some(EntryPoints {
                    path: default_entry,
                }),
                ..Default::default()
            }
        };

        let manifest_package = configuration.name.clone();

        debug!(
            "Resolved manifest package '{}' for directory '{}'",
            manifest_package, package_name
        );

        // Load tools and workflows using new architecture only
        let (tools, workflows) = self
            .load_tools_and_workflows(&configuration, &package_path, &manifest_package)
            .await?;

        // Load agents from configuration
        let agents = self
            .load_agents(&configuration, &package_path, &manifest_package)
            .await?;

        let package_artifact = PluginArtifact {
            name: manifest_package.clone(),
            path: package_path,
            configuration,
            tools,
            workflows,
            agents,
        };

        debug!(
            "Successfully loaded package: {} ({} tools, {} workflows, {} agents)",
            package_artifact.name,
            package_artifact.tools.len(),
            package_artifact.workflows.len(),
            package_artifact.agents.len()
        );

        Ok(package_artifact)
    }

    /// Load tools and workflows from package using new architecture only
    async fn load_tools_and_workflows(
        &self,
        configuration: &DistriServerConfig,
        package_path: &PathBuf,
        package_name: &str,
    ) -> Result<(Vec<PluginToolDefinition>, Vec<PluginWorkflowDefinition>)> {
        // Check for TypeScript package-level entry point (ts/index.ts)

        match &configuration.entrypoints {
            Some(distri_types::configuration::EntryPoints { path }) => {
                self.load_typescript_exports(configuration, package_path, package_name, path)
                    .await
            }
            _ => Ok((Vec::new(), Vec::new())),
        }
    }

    /// Load agents from configuration
    pub async fn load_agents(
        &self,
        configuration: &DistriServerConfig,
        package_path: &PathBuf,
        package_name: &str,
    ) -> Result<Vec<PluginAgentDefinition>> {
        let mut agents = Vec::new();

        if let Some(agent_paths) = &configuration.agents {
            for agent_path in agent_paths {
                let agent_file_path = package_path.join(agent_path);
                if agent_file_path.exists() {
                    // Parse the agent TOML file to get the full AgentDefinition
                    let agent_toml_content = fs::read_to_string(&agent_file_path).await?;
                    let mut agent_definition: StandardDefinition =
                        toml::from_str(&agent_toml_content).map_err(|e| {
                            anyhow!("Failed to parse agent TOML {}: {}", agent_path, e)
                        })?;

                    agent_definition.package_name = Some(package_name.to_string());

                    // Extract agent name from the file or use the name from the TOML
                    let agent_name = agent_definition.name.clone();

                    let agent_config =
                        distri_types::configuration::AgentConfig::StandardAgent(agent_definition);
                    let agent_dap_definition = PluginAgentDefinition {
                        name: agent_name,
                        package_name: package_name.to_string(),
                        description: agent_config.get_description().to_string(),
                        file_path: agent_file_path,
                        agent_config,
                    };
                    agents.push(agent_dap_definition);
                }
            }
        }

        Ok(agents)
    }

    async fn load_typescript_exports(
        &self,
        configuration: &DistriServerConfig,
        package_path: &PathBuf,
        package_name: &str,
        entrypoint: &str,
    ) -> Result<(Vec<PluginToolDefinition>, Vec<PluginWorkflowDefinition>)> {
        use distri_plugin_executor::executors::UnifiedPluginSystem;

        let resolved_entrypoint = resolve_entrypoint_path(package_path, entrypoint);

        debug!(
            "Loading TypeScript plugin {} from {:?} (entrypoint: {} -> resolved {})",
            package_name, package_path, entrypoint, resolved_entrypoint
        );

        let resolver = Arc::new(LocalPluginResolver::new(package_path.clone()));
        let context = PluginLoadContext {
            package_name: package_name.to_string(),
            entrypoint: Some(resolved_entrypoint.clone()),
            manifest: configuration.clone(),
            resolver,
        };

        let executor = UnifiedPluginSystem::new(Arc::new(MockOrchestrator));
        let plugin_id = executor.load_plugin(context).await?;
        let plugin_info = executor.get_plugin_info(&plugin_id).await?;

        let mut tools = Vec::new();
        let mut workflows = Vec::new();

        for integration in &plugin_info.integrations {
            for tool in &integration.tools {
                tools.push(PluginToolDefinition {
                    name: tool.name.clone(),
                    package_name: package_name.to_string(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                    auth: tool.auth.clone().or_else(|| integration.auth.clone()),
                });
            }
        }

        for workflow in &plugin_info.workflows {
            workflows.push(PluginWorkflowDefinition {
                name: workflow.name.clone(),
                package_name: package_name.to_string(),
                description: workflow.description.clone(),
                parameters: workflow.parameters.clone(),
                examples: workflow.examples.clone(),
            });
        }

        debug!(
            "Loaded {} tools and {} workflows from TypeScript plugin {}",
            tools.len(),
            workflows.len(),
            package_name
        );

        Ok((tools, workflows))
    }
}

fn resolve_entrypoint_path(package_path: &PathBuf, entrypoint: &str) -> String {
    use std::path::Path;

    let package_path = package_path.as_path();
    let entry_path = Path::new(entrypoint);

    // If an explicit extension is provided, respect it.
    if entry_path.extension().is_some() {
        return entrypoint.to_string();
    }

    let absolute = package_path.join(entry_path);
    if absolute.exists() {
        return entrypoint.to_string();
    }

    let ts_candidate = entry_path.with_extension("ts");
    if package_path.join(&ts_candidate).exists() {
        return ts_candidate.to_string_lossy().into_owned();
    }

    let mod_candidate = entry_path.join("mod.ts");
    if package_path.join(&mod_candidate).exists() {
        return mod_candidate.to_string_lossy().into_owned();
    }

    entrypoint.to_string()
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}
