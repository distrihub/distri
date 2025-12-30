use anyhow::{anyhow, Result};
use distri_types::configuration::DapPackageArtifact;
use rustyscript::{json_args, Module, Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::time::Instant;
use tracing::debug;

use crate::plugin::{DistriPlugin, ExecutionContext, ExecutionResult, ItemType, PluginType};

/// TypeScript plugin implementation using rustyscript
pub struct TypeScriptPlugin {
    name: String,
    package: DapPackageArtifact,
    runtime: Runtime,
}

impl TypeScriptPlugin {
    /// Create a new TypeScript plugin from a DAP package
    pub async fn new(package: DapPackageArtifact) -> Result<Self> {
        debug!("Creating TypeScript plugin for package: {}", package.name);

        // Create runtime
        let runtime_options = RuntimeOptions {
            ..Default::default()
        };
        let mut runtime = Runtime::new(runtime_options)
            .map_err(|e| anyhow!("Failed to create TypeScript runtime: {}", e))?;

        // Load base distri module
        let base_module_content = r#"
// Base classes for DAP tools and workflows
export interface DistriPlugin {
    tools: any;
    workflows: any;
}

export interface DistriTool {
    name: string;
    description: string;
    version: string;
    execute(toolCall: any, context: any): Promise<any>;
    getParameters(): any;
}

export interface DistriWorkflow {
    name: string;
    description: string;
    version: string;
    execute(params: any): Promise<any>;
    getParameters(): any;
}
"#;

        let base_module = Module::new("distri/base.ts", base_module_content);
        runtime
            .load_module(&base_module)
            .map_err(|e| anyhow!("Failed to load base module: {}", e))?;

        // Load package module
        let index_content = package
            .create_index_content()
            .map_err(|e| anyhow!("Failed to create index content for {}: {}", package.name, e))?;

        let module_name = format!("{}/index.ts", package.name);
        let package_module = Module::new(&module_name, &index_content);
        runtime
            .load_module(&package_module)
            .map_err(|e| anyhow!("Failed to load package module {}: {}", module_name, e))?;

        debug!(
            "TypeScript plugin {} initialized with {} tools and {} workflows",
            package.name,
            package.tools.len(),
            package.workflows.len()
        );

        Ok(Self {
            name: package.name.clone(),
            package,
            runtime,
        })
    }
}

#[async_trait::async_trait]
impl DistriPlugin for TypeScriptPlugin {
    fn get_name(&self) -> &String {
        &self.name
    }

    fn get_type(&self) -> PluginType {
        PluginType::TypeScript
    }

    fn get_info(&self) -> Value {
        json!({
            "name": self.name,
            "type": "typescript",
            "tools": self.get_tools(),
            "workflows": self.get_workflows(),
            "description": self.package.manifest.description,
            "version": self.package.manifest.version
        })
    }

    async fn execute(&self, item_name: &str, context: ExecutionContext) -> Result<ExecutionResult> {
        let start_time = Instant::now();

        debug!(
            "Executing {}/{} on TypeScript runtime",
            self.name, item_name
        );

        // Create execution context for the module
        let execution_context = json!({
            "agent_id": context.agent_id.as_deref().unwrap_or("unknown"),
            "session_id": context.session_id.as_deref().unwrap_or("unknown"),
            "task_id": context.task_id.as_deref().unwrap_or("unknown"),
            "run_id": context.run_id.as_deref().unwrap_or("unknown"),
            "environment": context.environment
        });

        // Execute the module
        // Note: We need to make runtime mutable for execute_module
        // This is a limitation we need to work around - perhaps by using Arc<Mutex<Runtime>>
        // For now, let's return an error indicating this needs to be fixed
        let execution_time_ms = start_time.elapsed().as_millis() as u64;

        // TODO: Fix mutability issue with runtime.execute_module
        // For now return a placeholder error
        Ok(ExecutionResult::error(
            "TypeScript execution not yet implemented due to runtime mutability".to_string(),
            execution_time_ms,
        ))
    }

    fn get_tools(&self) -> Vec<String> {
        self.package.tools.iter().map(|t| t.name.clone()).collect()
    }

    fn get_workflows(&self) -> Vec<String> {
        self.package
            .workflows
            .iter()
            .map(|w| w.name.clone())
            .collect()
    }
}

// Helper traits to check package content types
pub trait HasTypeScriptContent {
    fn has_typescript_content(&self) -> bool;
}

pub trait HasWasmContent {
    fn has_wasm_content(&self) -> bool;
}

impl HasTypeScriptContent for DapPackageArtifact {
    fn has_typescript_content(&self) -> bool {
        // Check if package has TypeScript files or tools/workflows
        !self.tools.is_empty() || !self.workflows.is_empty()
    }
}

impl HasWasmContent for DapPackageArtifact {
    fn has_wasm_content(&self) -> bool {
        // TODO: Check if package has WASM files
        // For now, return false since we don't have WASM detection logic
        false
    }
}
