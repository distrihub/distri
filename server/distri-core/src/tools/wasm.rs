use anyhow::{Context, Result};
use distri_plugin_executor::{PluginContext, PluginExecutor, UnifiedPluginSystem};
use distri_types::{Tool, ToolContext};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// WASM tool wrapper that loads and executes WASM-compiled tools
pub struct WasmTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub wasm_path: PathBuf,
    pub package_name: String,
    pub plugin_system: Option<Arc<UnifiedPluginSystem>>,
}

impl std::fmt::Debug for WasmTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("parameters", &self.parameters)
            .field("wasm_path", &self.wasm_path)
            .field("package_name", &self.package_name)
            .field("plugin_system", &"<UnifiedPluginSystem>")
            .finish()
    }
}

impl WasmTool {
    pub fn new(
        name: String,
        description: String,
        parameters: Value,
        wasm_path: PathBuf,
        package_name: String,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
            wasm_path,
            package_name,
            plugin_system: None,
        }
    }

    pub fn with_plugin_system(
        name: String,
        description: String,
        parameters: Value,
        wasm_path: PathBuf,
        package_name: String,
        plugin_system: Arc<UnifiedPluginSystem>,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
            wasm_path,
            package_name,
            plugin_system: Some(plugin_system),
        }
    }

    /// Load WASM module and get tool metadata
    pub async fn load_metadata(&self) -> Result<WasmToolMetadata> {
        // TODO: Implement WASM module loading to extract metadata
        // For now, return metadata based on the tool configuration
        Ok(WasmToolMetadata {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            version: "1.0.0".to_string(),
        })
    }
}

/// Metadata extracted from WASM tool module
#[derive(Debug, Clone)]
pub struct WasmToolMetadata {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub version: String,
}

#[async_trait::async_trait]
impl Tool for WasmTool {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_description(&self) -> String {
        self.description.clone()
    }

    fn get_parameters(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(
        &self,
        tool_call: distri_types::ToolCall,
        context: Arc<ToolContext>,
    ) -> Result<Vec<distri_types::Part>> {
        tracing::info!(
            "Executing WASM tool: {} from package: {}",
            self.name,
            self.package_name
        );

        if let Some(plugin_system) = &self.plugin_system {
            // Convert distri_types::ToolCall to dap_types::ToolCall

            // Convert to plugin executor format
            let execution_context = PluginContext {
                call_id: context.thread_id.clone(), // Use thread_id as call_id
                agent_id: Some(context.agent_id.clone()),
                session_id: Some(context.session_id.clone()),
                task_id: Some(context.task_id.clone()),
                run_id: Some(context.run_id.clone()),
                user_id: Some(context.user_id.clone()),
                params: serde_json::Value::Object(serde_json::Map::new()), // Empty params for now
                secrets: std::collections::HashMap::new(), // TODO: Load secrets if needed for WASM tools
                env_vars: None, // WASM tools don't have access to ExecutorContext env_vars
            };

            let result = plugin_system
                .execute_tool(&self.package_name, &tool_call, execution_context)
                .await?;

            Ok(vec![distri_types::Part::Data(result)])
        } else {
            Err(anyhow::anyhow!(
                "No plugin system available for WASM tool execution. Tool: {}, Package: {}",
                self.name,
                self.package_name
            ))
        }
    }
}

/// WASM tool loader for DAP packages
pub struct WasmToolLoader {
    root_path: PathBuf,
}

impl WasmToolLoader {
    pub fn new(dap_root: PathBuf) -> Self {
        Self {
            root_path: dap_root,
        }
    }

    /// Load WASM tools from a DAP package
    pub async fn load_tools_from_package(&self, package_name: &str) -> Result<Vec<Arc<WasmTool>>> {
        let package_path = self.root_path.join(package_name);
        let wasm_path = package_path.join("wasm");

        if !wasm_path.exists() {
            tracing::debug!("No wasm directory found for package: {}", package_name);
            return Ok(Vec::new());
        }

        let mut wasm_tools = Vec::new();

        // Scan for .wasm files in the wasm directory
        let entries = tokio::fs::read_dir(&wasm_path)
            .await
            .with_context(|| format!("Failed to read wasm directory: {:?}", wasm_path))?;

        let mut entries = entries;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if let Some(extension) = path.extension() {
                if extension == "wasm" {
                    if let Some(tool_name) = path.file_stem().and_then(|s| s.to_str()) {
                        // Create WASM tool instance
                        // TODO: Extract metadata from WASM module or companion manifest
                        let wasm_tool = WasmTool::new(
                            tool_name.to_string(),
                            format!("WASM tool: {}", tool_name),
                            serde_json::json!({
                                "type": "object",
                                "properties": {},
                                "required": []
                            }),
                            path.clone(),
                            package_name.to_string(),
                        );

                        wasm_tools.push(Arc::new(wasm_tool));
                        tracing::debug!(
                            "Discovered WASM tool: {} from package: {}",
                            tool_name,
                            package_name
                        );
                    }
                }
            }
        }

        Ok(wasm_tools)
    }

    /// Load all WASM tools from all DAP packages
    pub async fn load_all_tools(&self) -> Result<Vec<Arc<WasmTool>>> {
        let mut all_tools = Vec::new();

        if !self.root_path.exists() {
            tracing::debug!("DAP root directory not found: {:?}", self.root_path);
            return Ok(all_tools);
        }

        let mut entries = tokio::fs::read_dir(&self.root_path)
            .await
            .with_context(|| format!("Failed to read DAP root directory: {:?}", self.root_path))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_dir() {
                if let Some(package_name) = path.file_name().and_then(|s| s.to_str()) {
                    match self.load_tools_from_package(package_name).await {
                        Ok(mut package_tools) => {
                            all_tools.append(&mut package_tools);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load WASM tools from package {}: {}",
                                package_name,
                                e
                            );
                        }
                    }
                }
            }
        }

        tracing::debug!("Loaded {} WASM tools from DAP packages", all_tools.len());
        Ok(all_tools)
    }
}
