use anyhow::{anyhow, Context, Result};
use distri_types::ToolCall;
use distri_types::{configuration::format_plugin_module_key, OrchestratorTrait};
use rustyscript::{
    deno_core::ModuleId,
    worker::{
        DefaultWorker, DefaultWorkerOptions, DefaultWorkerQuery, DefaultWorkerResponse,
        InnerWorker, Worker,
    },
    Module, ModuleHandle, Runtime,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tracing::{debug, error};
use uuid::Uuid;

use crate::plugin_trait::PluginLoadContext;
use crate::{
    executors::ts_executor::{importer::DistriImportProvider, types::*},
    plugin_trait::{PluginContext, PluginExecutor, PluginFileResolver, PluginInfo},
};

pub struct PluginWorker {
    worker: Worker<PluginWorkerInner>,
}

#[derive(Clone, Debug)]
pub struct LoadedModule {
    module_id: ModuleId,
    module_path: String,
}

/// TypeScript plugin executor with runtime function support
pub struct TypeScriptPluginExecutor {
    id: Uuid,
    worker: Arc<Mutex<PluginWorker>>,
    tool_worker: Arc<Mutex<PluginWorker>>,
    module_ids: Arc<RwLock<HashMap<String, LoadedModule>>>,
    resolvers: Arc<RwLock<HashMap<String, Arc<dyn PluginFileResolver>>>>,
    session_user_map: Arc<RwLock<HashMap<String, String>>>,
}

pub struct PluginWorkerInner;

#[derive(Clone)]
pub struct TypeScriptPluginOptions {
    default_options: DefaultWorkerOptions,
    workflow_runtime: Arc<dyn OrchestratorTrait>,
    resolvers: Arc<RwLock<HashMap<String, Arc<dyn PluginFileResolver>>>>,
    session_user_map: Arc<RwLock<HashMap<String, String>>>,
}
impl InnerWorker for PluginWorkerInner {
    type Query = DefaultWorkerQuery;
    type Response = DefaultWorkerResponse;
    type RuntimeOptions = TypeScriptPluginOptions;
    type Runtime = (Runtime, std::collections::HashMap<ModuleId, ModuleHandle>);

    /// Initialize the runtime using the options provided
    fn init_runtime(options: Self::RuntimeOptions) -> Result<Self::Runtime, rustyscript::Error> {
        let default_options = options.default_options;

        // Create import provider for TypeScript path resolution
        let import_provider = DistriImportProvider::new(options.resolvers.clone());

        let mut runtime = Runtime::new(rustyscript::RuntimeOptions {
            default_entrypoint: default_options.default_entrypoint,
            timeout: default_options.timeout,
            shared_array_buffer_store: default_options.shared_array_buffer_store,
            startup_snapshot: default_options.startup_snapshot,
            import_provider: Some(Box::new(import_provider)),
            ..Default::default()
        })?;
        Self::register_workflow_functions(
            &mut runtime,
            options.workflow_runtime.clone(),
            options.session_user_map.clone(),
        )?;
        let modules = std::collections::HashMap::new();
        Ok((runtime, modules))
    }

    fn handle_query(runtime: &mut Self::Runtime, query: Self::Query) -> Self::Response {
        DefaultWorker::handle_query(runtime, query)
    }
}
impl TypeScriptPluginExecutor {
    /// Create a new plugin system
    pub fn new(workflow_runtime: Arc<dyn OrchestratorTrait>) -> Result<Self> {
        let id = Uuid::new_v4();
        debug!("🆔 Creating new TypeScript executor with ID: {}", id);

        let mut options = DefaultWorkerOptions::default();
        options.timeout = std::time::Duration::from_secs(30); // Increase timeout to 30 seconds

        let resolvers = Arc::new(RwLock::new(HashMap::new()));
        let session_user_map = Arc::new(RwLock::new(HashMap::new()));

        let plugin_options = TypeScriptPluginOptions {
            default_options: options,
            workflow_runtime,
            resolvers: resolvers.clone(),
            session_user_map: session_user_map.clone(),
        };
        let worker = Worker::new(plugin_options.clone())
            .map_err(|e| anyhow!("Failed to create worker: {}", e))?;

        let worker2 =
            Worker::new(plugin_options).map_err(|e| anyhow!("Failed to create worker: {}", e))?;

        let mut plugin_system = Self {
            id,
            worker: Arc::new(Mutex::new(PluginWorker { worker })),
            tool_worker: Arc::new(Mutex::new(PluginWorker { worker: worker2 })),
            module_ids: Arc::new(RwLock::new(HashMap::new())),
            resolvers,
            session_user_map,
        };

        plugin_system.load_distri_modules()?;
        Ok(plugin_system)
    }

    fn cache_session_user(&self, context: &PluginContext) {
        if let (Some(session_id), Some(user_id)) = (&context.session_id, &context.user_id) {
            if let Ok(mut guard) = self.session_user_map.write() {
                guard.insert(session_id.clone(), user_id.clone());
            }
        }
    }

    pub fn load_distri_modules(&mut self) -> Result<()> {
        let base_module = Module::new(DISTRI_BASE.0, DISTRI_BASE.1);
        self.load_module(DISTRI_BASE.0, &base_module)?;
        self.load_module_on_tool_worker(&base_module)?;

        let execute_module = Module::new(EXECUTE.0, EXECUTE.1);
        self.load_module(EXECUTE.0, &execute_module)?;
        self.load_module_on_tool_worker(&execute_module)?;

        Ok(())
    }

    pub fn load_module(&self, name: &str, module: &Module) -> Result<()> {
        let worker = self
            .worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire worker lock: {}", e))?;

        debug!("🔄 Loading module: {:?}", module.filename());

        let message = DefaultWorkerQuery::LoadModule(module.clone());

        match worker
            .worker
            .send_and_await(message)
            .map_err(|e| anyhow!("Worker communication failed: {}", e))?
        {
            DefaultWorkerResponse::ModuleId(module_id) => {
                self.module_ids
                    .write()
                    .map_err(|e| anyhow!("Failed to acquire write lock on module_ids: {}", e))?
                    .insert(
                        name.to_string(),
                        LoadedModule {
                            module_id,
                            module_path: name.to_string(),
                        },
                    );
                Ok(())
            }
            DefaultWorkerResponse::Error(e) => Err(anyhow!("Load plugin failed: {}", e)),
            _ => Err(anyhow!("Unexpected response from worker")),
        }
    }

    pub fn load_module_on_tool_worker(&self, module: &Module) -> Result<()> {
        let worker: std::sync::MutexGuard<'_, PluginWorker> = self
            .tool_worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire tool worker lock: {}", e))?;

        debug!("🔄 Loading module on tool worker: {:?}", module.filename());

        let message = DefaultWorkerQuery::LoadModule(module.clone());

        match worker
            .worker
            .send_and_await(message)
            .map_err(|e| anyhow!("Tool worker communication failed: {}", e))?
        {
            DefaultWorkerResponse::ModuleId(_module_id) => {
                // Module ID is already stored by load_module, no need to store again
                Ok(())
            }
            DefaultWorkerResponse::Error(e) => {
                Err(anyhow!("Load plugin on tool worker failed: {}", e))
            }
            _ => Err(anyhow!("Unexpected response from tool worker")),
        }
    }
    /// Load a plugin from a provided context (manifest + resolver)
    pub fn load_plugin_from_context(&self, context: &PluginLoadContext) -> Result<String> {
        let package_name = context.package_name.clone();
        let entrypoint = context
            .entrypoint
            .clone()
            .ok_or_else(|| anyhow!("Plugin {} missing TypeScript entrypoint", package_name))?;
        let normalized_entrypoint = entrypoint.trim_start_matches('/');

        debug!(
            "🔄 Loading TypeScript plugin {} with entrypoint {}",
            package_name, normalized_entrypoint
        );

        // Register resolver for importer
        {
            let mut guard = self
                .resolvers
                .write()
                .map_err(|e| anyhow!("Failed to register resolver: {}", e))?;
            guard.insert(package_name.clone(), context.resolver.clone());
        }

        // Fetch module source via resolver
        let source_bytes = context
            .resolver
            .read(normalized_entrypoint)
            .with_context(|| {
                format!(
                    "Failed to read entrypoint {} for plugin {}",
                    normalized_entrypoint, package_name
                )
            })?;
        let plugin_content =
            String::from_utf8(source_bytes).context("Entrypoint content is not valid UTF-8")?;

        let module_name = format_plugin_module_key(&package_name);
        let module_spec = format!("plugin://{}/{}", package_name, normalized_entrypoint);
        debug!("🔄 Loading plugin module spec: {}", module_spec);

        // Load module on primary worker
        let worker = self
            .worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire worker lock: {}", e))?;
        let plugin_module = rustyscript::Module::new(&module_spec, &plugin_content);
        let result = worker
            .worker
            .send_and_await(DefaultWorkerQuery::LoadModule(plugin_module))?;

        let module_id = match result {
            DefaultWorkerResponse::ModuleId(id) => id,
            DefaultWorkerResponse::Error(e) => {
                return Err(anyhow!("Failed to load module {}: {}", module_spec, e))
            }
            _ => return Err(anyhow!("Unexpected response from worker")),
        };
        drop(worker);

        // Load module on tool worker for tool execution context
        let plugin_module_for_tool = rustyscript::Module::new(&module_spec, &plugin_content);
        let tool_worker = self
            .tool_worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire tool worker lock: {}", e))?;
        let tool_result = tool_worker
            .worker
            .send_and_await(DefaultWorkerQuery::LoadModule(plugin_module_for_tool))?;
        if let DefaultWorkerResponse::Error(e) = tool_result {
            return Err(anyhow!("Failed to load module on tool worker: {}", e));
        }
        drop(tool_worker);

        self.module_ids
            .write()
            .map_err(|e| anyhow!("Failed to track module IDs: {}", e))?
            .insert(
                module_name.clone(),
                LoadedModule {
                    module_id,
                    module_path: module_spec.clone(),
                },
            );

        // Validate exports
        match self.get_plugin_info_value(&module_name) {
            Ok(plugin_info) => {
                let tool_count = plugin_info
                    .get("tools")
                    .and_then(|value| value.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or_default();
                let workflow_count = plugin_info
                    .get("workflows")
                    .and_then(|value| value.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or_default();
                debug!(
                    "✅ Plugin {} registered with {} tools and {} workflows",
                    package_name, tool_count, workflow_count
                );
            }
            Err(e) => {
                error!("❌ Plugin validation failed for {}: {}", package_name, e);
                return Err(anyhow!("Plugin validation failed: {}", e));
            }
        }

        Ok(package_name)
    }

    pub fn get_plugin_info_value(&self, package_name: &str) -> Result<Value> {
        let worker = self
            .worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire worker lock: {}", e))?;
        let loaded_module = {
            let module_ids = self
                .module_ids
                .read()
                .map_err(|e| anyhow!("Failed to acquire read lock on module_ids: {}", e))?;
            let id = module_ids.get(package_name);
            id.cloned()
        }
        .ok_or_else(|| anyhow!("Plugin {} not loaded", package_name))?;

        // Get the raw plugin value
        let result: DefaultWorkerResponse = worker
            .worker
            .send_and_await(DefaultWorkerQuery::GetValue(
                Some(loaded_module.module_id),
                "default".to_string(),
            ))
            .map_err(|e| anyhow!("Failed to get plugin info: {}", e))?;

        let raw_plugin = match result {
            DefaultWorkerResponse::Value(value) => value,
            DefaultWorkerResponse::Error(e) => {
                return Err(anyhow!("Failed to get plugin info: {}", e))
            }
            _ => return Err(anyhow!("Unexpected response from worker")),
        };

        // Process the plugin to normalize parameters field
        let processed_result: DefaultWorkerResponse = worker
            .worker
            .send_and_await(DefaultWorkerQuery::CallFunction(
                None, // Call global function
                "processPlugin".to_string(),
                vec![raw_plugin],
            ))
            .map_err(|e| anyhow!("Failed to process plugin: {}", e))?;

        match processed_result {
            DefaultWorkerResponse::Value(value) => Ok(value),
            DefaultWorkerResponse::Error(e) => Err(anyhow!("Failed to process plugin: {}", e)),
            _ => Err(anyhow!("Unexpected response from processPlugin")),
        }
    }

    /// Execute a tool with the new flattened parameter structure
    pub async fn execute_tool_fn(
        &self,
        package_name: &str,
        tool_call: &ToolCall,
        context: PluginContext,
    ) -> Result<Value> {
        self.cache_session_user(&context);
        println!("plugin_context: {context:?}");
        let PluginContext {
            call_id,
            agent_id,
            session_id,
            task_id,
            run_id,
            user_id,
            params,
            secrets,
            env_vars,
        } = context;

        let call_id_clone = call_id.clone();
        let agent_id_clone = agent_id.clone();
        let session_id_clone = session_id.clone();
        let task_id_clone = task_id.clone();
        let run_id_clone = run_id.clone();
        let user_id_clone = user_id.clone();

        let worker = self
            .tool_worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire tool worker lock: {}", e))?;

        // Prepare execution context with env_vars and secrets
        let execution_context = serde_json::json!({
            "callId": call_id_clone,
            "call_id": call_id,
            "agentId": agent_id_clone,
            "agent_id": agent_id,
            "sessionId": session_id_clone,
            "session_id": session_id,
            "taskId": task_id_clone,
            "task_id": task_id,
            "runId": run_id_clone,
            "run_id": run_id,
            "userId": user_id_clone,
            "user_id": user_id,
            "params": params,
            "secrets": secrets,
            "env_vars": env_vars,
        });

        // Get module IDs
        let (execute_module, plugin_module) = {
            let module_ids = self
                .module_ids
                .read()
                .map_err(|e| anyhow!("Failed to acquire read lock on module_ids: {}", e))?;
            let plugin_key = format_plugin_module_key(&package_name);
            let plugin_module = module_ids.get(&plugin_key).cloned();
            let execute_module = module_ids.get(EXECUTE.0).cloned();

            match (execute_module, plugin_module) {
                (Some(execute_module), Some(plugin_module)) => (execute_module, plugin_module),
                (x, y) => {
                    tracing::debug!("Module IDs: {:?}", module_ids);
                    return Err(anyhow!(
                        "Error loading modules in execute: Execute : {} or Plugin:{}: {}",
                        x.is_some(),
                        plugin_key,
                        y.is_some(),
                    ));
                }
            }
        };

        // Call the TypeScript executeTool function with separate arguments
        let result: DefaultWorkerResponse = worker
            .worker
            .send_and_await(DefaultWorkerQuery::CallFunction(
                Some(execute_module.module_id),
                "executeTool".to_string(),
                vec![
                    plugin_module.module_path.clone().into(),
                    serde_json::to_value(tool_call)?,
                    execution_context,
                ],
            ))
            .map_err(|e| anyhow!("Failed to execute tool: {}", e))?;

        match result {
            DefaultWorkerResponse::Value(value) => Ok(value),
            DefaultWorkerResponse::Error(e) => Err(anyhow!("Error executing plugin: {}", e)),
            _ => Err(anyhow!("Unexpected response from worker")),
        }
    }

    /// Execute a tool or workflow on a loaded plugin
    pub async fn execute_fn(
        &self,
        package_name: &str,
        fn_call: PluginFunctionCall,
        context: PluginContext,
    ) -> Result<Value> {
        self.cache_session_user(&context);

        let PluginContext {
            call_id,
            agent_id,
            session_id,
            task_id,
            run_id,
            user_id,
            params,
            secrets,
            env_vars,
        } = context;

        let agent_id_value = agent_id.unwrap_or_else(|| "unknown".to_string());
        let session_id_value = session_id.unwrap_or_else(|| "unknown".to_string());
        let task_id_value = task_id.unwrap_or_else(|| "unknown".to_string());
        let run_id_value = run_id.unwrap_or_else(|| "unknown".to_string());
        let user_id_value = user_id.unwrap_or_else(|| "unknown".to_string());
        let call_id_clone = call_id.clone();

        let worker = self
            .worker
            .lock()
            .map_err(|e| anyhow!("Failed to acquire worker lock: {}", e))?;

        // Prepare execution context
        let execution_context = serde_json::json!({
            "call_id": call_id_clone,
            "callId": call_id,
            "agent_id": agent_id_value.clone(),
            "agentId": agent_id_value,
            "session_id": session_id_value.clone(),
            "sessionId": session_id_value,
            "task_id": task_id_value.clone(),
            "taskId": task_id_value,
            "run_id": run_id_value.clone(),
            "runId": run_id_value,
            "user_id": user_id_value.clone(),
            "userId": user_id_value,
            "params": params,
            "secrets": secrets,
            "env_vars": env_vars,
        });

        let (execute_module, plugin_module) = {
            let module_ids = self
                .module_ids
                .read()
                .map_err(|e| anyhow!("Failed to acquire read lock on module_ids: {}", e))?;
            debug!("execute_fn module_ids: {:#?}", module_ids);
            let plugin_key = format_plugin_module_key(&package_name);
            debug!("Looking for plugin module with key: {}", plugin_key);
            let plugin_module = module_ids.get(&plugin_key).cloned();
            let execute_module = module_ids.get(EXECUTE.0).cloned();

            let execute_found = execute_module.is_some();
            let plugin_found = plugin_module.is_some();
            debug!("Execute module found: {}", execute_found);
            debug!("Plugin module found: {}", plugin_found);

            match (execute_module, plugin_module) {
                (Some(execute_module), Some(plugin_module)) => (execute_module, plugin_module),
                _ => return Err(anyhow!("Execute module or plugin module not loaded. Execute module: {}, Plugin module: {} (key: {})", 
                    execute_found, plugin_found, plugin_key)),
            }
        };

        let result: DefaultWorkerResponse = worker
            .worker
            .send_and_await(DefaultWorkerQuery::CallFunction(
                Some(execute_module.module_id),
                fn_call.function_name,
                vec![
                    plugin_module.module_path.clone().into(),
                    fn_call.args,
                    execution_context,
                ],
            ))
            .map_err(|e| {
                anyhow!(
                    "Failed to execute plugin: {}, module_id: {}",
                    e,
                    execute_module.module_id
                )
            })?;

        match result {
            DefaultWorkerResponse::Value(value) => Ok(value),
            DefaultWorkerResponse::Error(e) => Err(anyhow!("Failed to execute plugin: {}", e)),
            _ => Err(anyhow!("Unexpected response from worker")),
        }
    }
}

#[async_trait::async_trait]
impl PluginExecutor for TypeScriptPluginExecutor {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn cleanup(&self) {
        debug!("🧹 TypeScript plugin cleanup... Closing workers");
        self.worker.lock().unwrap().worker.shutdown();
        self.tool_worker.lock().unwrap().worker.shutdown();
    }
    async fn load_plugin(&self, context: PluginLoadContext) -> Result<String> {
        debug!(
            "📦 TypeScript executor {} loading plugin {}",
            self.id, context.package_name
        );
        TypeScriptPluginExecutor::load_plugin_from_context(self, &context)
    }

    async fn get_plugin_info(&self, package_name: &str) -> Result<PluginInfo> {
        let module_key = format_plugin_module_key(package_name);
        let plugin_value = self.get_plugin_info_value(&module_key)?;

        // Use serde to parse plugin info directly
        let mut plugin_info: PluginInfo = serde_json::from_value(plugin_value)
            .map_err(|e| anyhow!("Failed to parse plugin info: {}", e))?;

        // Set the package name
        plugin_info.package_name = package_name.to_string();

        Ok(plugin_info)
    }

    async fn execute_tool(
        &self,
        package_name: &str,
        tool_call: &ToolCall,
        context: PluginContext,
    ) -> Result<Value> {
        debug!(
            "🔧 execute_tool called: package={}, tool={}",
            package_name, tool_call.tool_name
        );
        self.execute_tool_fn(package_name, tool_call, context).await
    }

    async fn execute_workflow(
        &self,
        package_name: &str,
        workflow_name: &str,
        input: Value,
        context: PluginContext,
    ) -> Result<Value> {
        let workflow_call = PluginWorkflowCall {
            workflow_call_id: context.call_id.clone(),
            workflow_name: workflow_name.to_string(),
            input: input.clone(),
        };
        let fn_call = PluginFunctionCall {
            function_name: "executeWorkflow".to_string(),
            args: serde_json::to_value(&workflow_call)?,
        };

        debug!(
            "execute_workflow fn_call, package_name: {}, workflow_name: {}, fn_call: {:?}",
            package_name, workflow_name, fn_call
        );

        self.execute_fn(package_name, fn_call, context).await
    }

    fn get_loaded_plugins(&self) -> Vec<String> {
        match self.module_ids.read() {
            Ok(module_ids) => module_ids.keys().cloned().collect(),
            Err(_) => Vec::new(), // Return empty vec if lock is poisoned
        }
    }
}

impl PluginWorkerInner {
    /// Register workflow functions on the runtime
    fn register_workflow_functions(
        runtime: &mut Runtime,
        workflow_runtime: Arc<dyn OrchestratorTrait>,
        session_user_map: Arc<RwLock<HashMap<String, String>>>,
    ) -> Result<()> {
        // Register callAgent async function
        {
            let workflow_runtime = Arc::clone(&workflow_runtime);
            runtime.register_async_function("callAgent", move |args| {
                let workflow_runtime = workflow_runtime.clone();

                Box::pin(async move {
                    let params: CallAgentParams = args
                        .first()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("Missing parameters".to_string())
                        })
                        .and_then(|v| {
                            serde_json::from_value(v.clone()).map_err(|e| {
                                rustyscript::Error::Runtime(format!("Invalid parameters: {}", e))
                            })
                        })?;

                    debug!("callAgent called: {} -> {}", params.agent_name, params.task);

                    workflow_runtime
                        .call_agent(&params.session_id, &params.agent_name, &params.task)
                        .await
                        .map(serde_json::Value::String)
                        .map_err(|e| rustyscript::Error::Runtime(e.to_string()))
                })
            })?;
        }

        // Register callTool async function
        {
            let workflow_runtime1 = workflow_runtime.clone();
            let session_user_map = Arc::clone(&session_user_map);
            runtime.register_async_function("callTool", move |args| {
                let workflow_runtime = workflow_runtime1.clone();
                let session_user_map = Arc::clone(&session_user_map);
                Box::pin(async move {
                    let params: CallToolParams = args
                        .first()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("Missing parameters".to_string())
                        })
                        .and_then(|v| {
                            serde_json::from_value(v.clone()).map_err(|e| {
                                rustyscript::Error::Runtime(format!("Invalid parameters: {}", e))
                            })
                        })?;

                    let CallToolParams {
                        session_id,
                        user_id,
                        package_name: _,
                        tool_name,
                        input,
                    } = params;

                    debug!("callTool called: {} with params: {:?}", tool_name, input);

                    let resolved_user_id = user_id.filter(|id| !id.is_empty()).or_else(|| {
                        session_user_map
                            .read()
                            .ok()
                            .and_then(|map| map.get(&session_id).cloned())
                    });

                    let user_id = resolved_user_id.ok_or_else(|| {
                        rustyscript::Error::Runtime(
                            "Invalid parameters: missing field `user_id`".to_string(),
                        )
                    })?;

                    if let Ok(mut guard) = session_user_map.write() {
                        guard.insert(session_id.clone(), user_id.clone());
                    }

                    workflow_runtime
                        .call_tool(
                            &session_id,
                            &user_id,
                            &ToolCall {
                                tool_call_id: Uuid::new_v4().to_string(),
                                tool_name,
                                input,
                            },
                        )
                        .await
                        .map_err(|e| rustyscript::Error::Runtime(e.to_string()))
                })
            })?;
        }

        // Register getSessionValue async function
        {
            let workflow_runtime2 = workflow_runtime.clone();
            runtime.register_async_function("getSessionValue", move |args| {
                let workflow_runtime = workflow_runtime2.clone();

                Box::pin(async move {
                    let params: GetSessionValueParams = args
                        .first()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("Missing parameters".to_string())
                        })
                        .and_then(|v| {
                            serde_json::from_value(v.clone()).map_err(|e| {
                                rustyscript::Error::Runtime(format!("Invalid parameters: {}", e))
                            })
                        })?;

                    debug!(
                        "getSessionValue called: {} -> {}",
                        params.session_id, params.key
                    );

                    workflow_runtime
                        .get_session_value(&params.session_id, &params.key)
                        .await
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("Session value not found".to_string())
                        })
                })
            })?;
        }

        // Register setSessionValue async function
        {
            let workflow_runtime = Arc::clone(&workflow_runtime);
            runtime.register_async_function("setSessionValue", move |args| {
                let workflow_runtime = workflow_runtime.clone();

                Box::pin(async move {
                    let params: SetSessionValueParams = args
                        .first()
                        .ok_or_else(|| {
                            rustyscript::Error::Runtime("Missing parameters".to_string())
                        })
                        .and_then(|v| {
                            serde_json::from_value(v.clone()).map_err(|e| {
                                rustyscript::Error::Runtime(format!("Invalid parameters: {}", e))
                            })
                        })?;

                    debug!(
                        "setSessionValue called: {} -> {} = {:?}",
                        params.session_id, params.key, params.value
                    );

                    workflow_runtime
                        .set_session_value(&params.session_id, &params.key, params.value)
                        .await
                        .map_err(|e| rustyscript::Error::Runtime(e.to_string()))?;

                    Ok::<_, rustyscript::Error>(serde_json::Value::Null)
                })
            })?;
        }

        Ok(())
    }
}
