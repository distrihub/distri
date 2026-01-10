use anyhow::{anyhow, Result};
use distri_types::ToolCall;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};
use uuid::Uuid;

#[cfg(feature = "typescript")]
use crate::executors::TypeScriptPluginExecutor;

use crate::plugin_trait::{PluginExecutor, PluginInfo, PluginLoadContext};
use crate::OrchestratorTrait;
use crate::PluginContext;

/// Unified plugin system that dynamically initializes executors based on plugin type
pub struct UnifiedPluginSystem {
    id: Uuid,
    // Use Arc to allow cloning executors for concurrent access
    executors: std::sync::RwLock<Option<std::sync::Arc<dyn PluginExecutor + Send + Sync>>>,
    orchestrator: std::sync::RwLock<Arc<dyn OrchestratorTrait>>,
}

impl UnifiedPluginSystem {
    /// Create a new unified plugin system with orchestrator
    pub fn new(orchestrator: Arc<dyn OrchestratorTrait>) -> Self {
        let id = Uuid::new_v4();
        debug!("🆔 Creating new UnifiedPluginSystem with ID: {}", id);

        Self {
            id,
            executors: Default::default(),
            orchestrator: std::sync::RwLock::new(orchestrator),
        }
    }

    /// Get or create an executor for the given plugin type
    fn get_or_create_executor(&self) -> Result<()> {
        let executors = self.executors.read().unwrap();
        if executors.is_some() {
            return Ok(());
        }
        drop(executors);

        // Create executor for this type if it doesn't exist
        let current_orchestrator = self.orchestrator.read().unwrap().clone();
        debug!("Creating TypeScript executor with orchestrator");
        let executor: std::sync::Arc<dyn PluginExecutor + Send + Sync> =
            std::sync::Arc::new(TypeScriptPluginExecutor::new(current_orchestrator)?);

        let mut executors = self.executors.write().unwrap();
        *executors = Some(executor);
        Ok(())
    }

    pub fn set_orchestrator(&self, orchestrator: Arc<dyn OrchestratorTrait>) {
        {
            let mut guard = self.orchestrator.write().unwrap();
            *guard = orchestrator;
        }

        // Drop existing executors so they will be recreated with the new orchestrator
        *self.executors.write().unwrap() = None;
    }

    /// Initialize the plugin system by eagerly creating the executor.
    /// Call this after set_orchestrator to ensure the executor is ready.
    pub fn initialize(&self) -> Result<()> {
        debug!("🔄 UnifiedPluginSystem {} initializing executor eagerly", self.id);
        self.get_or_create_executor()?;
        info!("✅ UnifiedPluginSystem {} executor initialized", self.id);
        Ok(())
    }

    /// Infer build command for a plugin based on its type and structure
    pub fn infer_build_command(package_path: &Path) -> Result<Option<String>> {
        // Check for package.json for npm/pnpm build
        let package_json = package_path.join("package.json");
        if package_json.exists() {
            // Check if pnpm or npm
            let lockfile_pnpm = package_path.join("pnpm-lock.yaml");
            let lockfile_npm = package_path.join("package-lock.json");

            if lockfile_pnpm.exists() {
                Ok(Some("pnpm run build".to_string()))
            } else if lockfile_npm.exists() {
                Ok(Some("npm run build".to_string()))
            } else {
                Ok(Some("npm install && npm run build".to_string()))
            }
        } else {
            // Pure TypeScript, no build needed typically
            Ok(None)
        }
    }

    /// Build a plugin using inferred or configured build command
    pub async fn build_plugin(&self, package_path: &Path) -> Result<()> {
        info!("🔨 Building plugin : {:?}", package_path);

        // Check for explicit build command in distri.toml first
        let distri_toml_path = package_path.join("distri.toml");

        let explicit_build_cmd = if distri_toml_path.exists() {
            let content = std::fs::read_to_string(&distri_toml_path)?;
            let config: toml::Value = toml::from_str(&content)?;
            config
                .get("build")
                .and_then(|b| b.get("command"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        let build_command = if let Some(explicit_cmd) = explicit_build_cmd {
            info!("📋 Using explicit build command: {}", explicit_cmd);
            explicit_cmd
        } else if let Some(inferred_cmd) = Self::infer_build_command(package_path)? {
            info!("🔍 Using inferred build command: {}", inferred_cmd);
            inferred_cmd
        } else {
            info!("ℹ️ No build command needed for this plugin");
            return Ok(());
        };

        // Execute build command
        info!("⚙️ Executing build command: {}", build_command);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&build_command)
            .current_dir(package_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Build failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        info!("✅ Build completed successfully: {}", stdout);
        Ok(())
    }
}

#[async_trait::async_trait]
impl PluginExecutor for UnifiedPluginSystem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    async fn load_plugin(&self, context: PluginLoadContext) -> Result<String> {
        self.load_plugin(context).await
    }

    async fn get_plugin_info(&self, package_name: &str) -> Result<PluginInfo> {
        let executor = {
            let executors = self.executors.read().unwrap();
            let exec = executors.as_ref().ok_or_else(|| {
                anyhow!(
                    "No executor found for package in get_plugin_info: {}",
                    package_name
                )
            })?;

            // Clone the Arc to avoid borrowing across await
            exec.clone()
        };

        executor.get_plugin_info(package_name).await
    }
    fn cleanup(&self) {
        let executors = self.executors.read().unwrap();
        if let Some(e) = executors.as_ref() {
            e.cleanup();
        }
    }

    async fn execute_tool(
        &self,
        package_name: &str,
        tool_call: &ToolCall,
        context: PluginContext,
    ) -> Result<Value> {
        debug!(
            "🔧 UnifiedPluginSystem {} executing tool: {}/{}",
            self.id, package_name, tool_call.tool_name
        );

        let executor = {
            let executors = self.executors.read().unwrap();
            if let Some(exec) = executors.as_ref() {
                // Executor exists, clone it
                exec.clone()
            } else {
                // Need to create executor, drop read lock first
                drop(executors);
                // Ensure we have an executor for this plugin type
                self.get_or_create_executor()?;

                // Now get the executor with a new read lock
                let executors = self.executors.read().unwrap();
                executors
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow!("Executor creation failed for package: {}", package_name)
                    })?
                    .clone()
            }
        };

        executor
            .execute_tool(package_name, tool_call, context)
            .await
    }

    async fn execute_workflow(
        &self,
        package_name: &str,
        workflow_name: &str,
        params: Value,
        context: PluginContext,
    ) -> Result<Value> {
        // Get executor (create if needed)
        let executor = {
            let executors = self.executors.read().unwrap();
            if let Some(exec) = executors.as_ref() {
                // Executor exists, clone it
                exec.clone()
            } else {
                // Need to create executor, drop read lock first
                drop(executors);
                // Ensure we have an executor for this plugin type
                self.get_or_create_executor()?;

                // Now get the executor with a new read lock
                let executors = self.executors.read().unwrap();
                executors
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow!("Executor creation failed for package: {}", package_name)
                    })?
                    .clone()
            }
        };

        info!("🔄 Executing workflow {}/{} ", package_name, workflow_name);

        executor
            .execute_workflow(package_name, workflow_name, params, context)
            .await
    }

    fn get_loaded_plugins(&self) -> Vec<String> {
        let mut plugins = Vec::new();
        let executors = self.executors.read().unwrap();

        if let Some(e) = executors.as_ref() {
            plugins.extend(e.get_loaded_plugins());
        }

        plugins
    }
}

impl UnifiedPluginSystem {
    pub async fn load_plugin(&self, context: PluginLoadContext) -> Result<String> {
        debug!(
            "🔄 UnifiedPluginSystem {} loading plugin: {} ",
            self.id, context.package_name
        );

        // Ensure we have an executor for this plugin type
        self.get_or_create_executor()?;

        // Load using appropriate executor
        let package_name = {
            let executor = {
                let executors = self.executors.read().unwrap();
                let exec = executors.as_ref().ok_or_else(|| {
                    anyhow!("No executor found for type: {:?}", context.package_name)
                })?;
                // Clone the Arc to avoid borrowing across await
                exec.clone()
            };

            // Drop the lock before awaiting
            executor.load_plugin(context.clone()).await?
        };
        Ok(package_name)
    }
}
