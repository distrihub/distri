use crate::{
    agent::{
        browser_sessions::BrowserSessions,
        plugin_registry::{PluginOptions, PluginRegistry},
        prompt_registry::PromptRegistry,
        todos::TodosTool,
        AgentEventType, BaseAgent, CoordinatorMessage, InvokeResult,
    },
    llm::LLMExecutor,
    servers::registry::McpServerRegistry,
    tools::Tool,
    types::{CreateThreadRequest, Message, Thread, ThreadSummary, UpdateThreadRequest},
    AgentError, HookRegistry,
};

use super::ExecutorContext;
use crate::agent::hooks::inline::InlineHook;
use distri_auth::ProviderRegistry;
use distri_filesystem::FileSystem;
use distri_stores::{initialize_stores, InitializedStores};
pub use distri_stores::{workflow::InMemoryWorkflowStore, AgentStore, ThreadStore};
use distri_types::stores::{PromptTemplateStore, SecretStore};
use distri_types::{
    auth::OAuthHandler, LlmDefinition, ModelSettings, Part, ServerMetadataWrapper, ToolCall,
    ToolsConfig,
};
use distri_types::{
    browser::DistriBrowserConfig,
    configuration::{
        is_namespaced_plugin_id, namespace_plugin_item, split_namespaced_plugin_id,
        CustomAgentDefinition, CustomAgentExample, DistriServerConfig, StoreConfig,
    },
    HookMutation,
};
use distri_types::{
    configuration::{DefinitionOverrides, ObjectStorageConfig},
    LLmContext, OrchestratorTrait,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

pub const SKILL_STORAGE_ROOT: &str = "storage/skills";

// Message types for coordinator communication

#[derive(Clone)]
pub struct AgentOrchestrator {
    pub tool_auth_handler: Arc<OAuthHandler>,
    pub mcp_registry: Arc<RwLock<McpServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub workspace_filesystem: Arc<FileSystem>,
    pub session_filesystem: Arc<FileSystem>,
    pub session_root_prefix: Option<String>,
    pub browser_config: Arc<RwLock<DistriBrowserConfig>>,
    pub browser_sessions: Arc<BrowserSessions>,
    pub additional_tools: Arc<RwLock<HashMap<String, Vec<Arc<dyn Tool>>>>>,
    pub plugin_registry: Arc<PluginRegistry>,
    pub plugin_tools: Arc<RwLock<HashMap<String, Vec<Arc<dyn Tool>>>>>,
    pub workspace_path: std::path::PathBuf,
    pub prompt_registry: Arc<PromptRegistry>,
    /// Store configuration for creating new session stores
    pub store_config: StoreConfig,
    /// All stores - use this instead of individual store fields
    pub stores: InitializedStores,
    pub configuration: Arc<RwLock<DistriServerConfig>>,
    pub default_model_settings: Arc<RwLock<ModelSettings>>,
    pub default_analysis_model_settings: Arc<RwLock<ModelSettings>>,
    pub hooks: Arc<RwLock<HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>>>,
    pub inline_hooks: Arc<dashmap::DashMap<String, tokio::sync::oneshot::Sender<HookMutation>>>,
    pub hook_registry: HookRegistry,
}

impl std::fmt::Debug for AgentOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOrchestrator").finish()
    }
}

impl Drop for AgentOrchestrator {
    fn drop(&mut self) {
        tracing::debug!("🌐 AgentOrchestrator dropping, cleaning up resources...");
        tracing::debug!("🌐 AgentOrchestrator cleanup completed");
    }
}

#[derive(Default)]
pub struct AgentOrchestratorBuilder {
    registry: Option<Arc<RwLock<McpServerRegistry>>>,
    additional_tools: Option<HashMap<String, Vec<Arc<dyn Tool>>>>,
    plugin_registry: Option<Arc<PluginRegistry>>,
    plugin_tools: Option<HashMap<String, Vec<Arc<dyn Tool>>>>,
    workspace_filesystem: Option<Arc<FileSystem>>,
    workspace_path: Option<std::path::PathBuf>,
    browser_config: Option<DistriBrowserConfig>,
    browser_sessions: Option<Arc<BrowserSessions>>,
    stores: Option<InitializedStores>,
    prompt_registry: Option<Arc<PromptRegistry>>,
    store_config: Option<StoreConfig>,
    configuration: Option<Arc<RwLock<DistriServerConfig>>>,
    default_model_settings: Option<ModelSettings>,
    default_analysis_model_settings: Option<ModelSettings>,
    hooks: Option<HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>>,
}

impl AgentOrchestratorBuilder {
    pub fn with_registry(mut self, registry: Arc<RwLock<McpServerRegistry>>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_additional_tools(
        mut self,
        additional_tools: HashMap<String, Vec<Arc<dyn Tool>>>,
    ) -> Self {
        self.additional_tools = Some(additional_tools);
        self
    }

    pub fn with_plugin_registry(mut self, plugin_registry: Arc<PluginRegistry>) -> Self {
        self.plugin_registry = Some(plugin_registry);
        self
    }

    pub fn with_plugin_tools(mut self, plugin_tools: HashMap<String, Vec<Arc<dyn Tool>>>) -> Self {
        self.plugin_tools = Some(plugin_tools);
        self
    }

    pub fn with_workspace_path(mut self, workspace_path: std::path::PathBuf) -> Self {
        self.workspace_path = Some(workspace_path);
        self
    }

    pub fn with_prompt_registry(mut self, prompt_registry: Arc<PromptRegistry>) -> Self {
        self.prompt_registry = Some(prompt_registry);
        self
    }

    pub fn with_prompt_template_store(mut self, store: Arc<dyn PromptTemplateStore>) -> Self {
        if let Some(stores) = &mut self.stores {
            stores.prompt_template_store = Some(store);
        } else {
            // This is a bit awkward since stores is Option, 
            // but build() will handle it if stores is None initially
        }
        self
    }

    pub fn with_secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        if let Some(stores) = &mut self.stores {
            stores.secret_store = Some(store);
        }
        self
    }

    pub fn with_stores(mut self, stores: InitializedStores) -> Self {
        self.stores = Some(stores);
        self
    }

    pub fn with_store_config(mut self, store_config: StoreConfig) -> Self {
        self.store_config = Some(store_config);
        self
    }

    pub fn with_configuration(mut self, configuration: Arc<RwLock<DistriServerConfig>>) -> Self {
        self.configuration = Some(configuration);
        self
    }

    pub fn with_workspace_file_system(mut self, file_system: Arc<FileSystem>) -> Self {
        self.workspace_filesystem = Some(file_system);
        self
    }

    pub fn with_browser_config(mut self, browser_config: DistriBrowserConfig) -> Self {
        self.browser_config = Some(browser_config);
        self
    }

    pub fn with_default_model_settings(mut self, model_settings: ModelSettings) -> Self {
        self.default_model_settings = Some(model_settings);
        self
    }

    pub fn with_default_analysis_model_settings(mut self, model_settings: ModelSettings) -> Self {
        self.default_analysis_model_settings = Some(model_settings);
        self
    }

    pub fn with_hooks(
        mut self,
        hooks: HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>,
    ) -> Self {
        self.hooks = Some(hooks);
        self
    }

    pub async fn build(self) -> anyhow::Result<AgentOrchestrator> {
        let (coordinator_tx, coordinator_rx) = mpsc::channel(10000);
        let browser_config = self.browser_config.unwrap_or_default();

        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(RwLock::new(McpServerRegistry::new())));

        let store_config = self.store_config.unwrap_or_default();
        let stores = match self.stores {
            Some(stores) => stores,
            None => initialize_stores(&store_config).await?,
        };
        let hooks_map = self.hooks.clone().unwrap_or_else(|| {
            let mut map: HashMap<String, Arc<dyn crate::agent::types::AgentHooks>> = HashMap::new();
            map.insert(
                "inline".to_string(),
                Arc::new(InlineHook::default()) as Arc<dyn crate::agent::types::AgentHooks>,
            );
            map
        });
        let hooks = Arc::new(RwLock::new(hooks_map));
        let workspace_path = self
            .workspace_path
            .unwrap_or_else(|| std::path::PathBuf::from("/"));

        let configuration = self
            .configuration
            .unwrap_or_else(|| Arc::new(RwLock::new(DistriServerConfig::default())));
        let filesystem_config = {
            let cfg_guard = configuration.read().await;
            cfg_guard.filesystem.clone()
        };

        let plugin_registry = if let Some(registry) = self.plugin_registry {
            registry
        } else {
            let registry = PluginRegistry::new(stores.plugin_store.clone())?;

            let mut plugin_options = PluginOptions::default();
            plugin_options.object_store =
                filesystem_config
                    .clone()
                    .unwrap_or(ObjectStorageConfig::FileSystem {
                        base_path: workspace_path.to_string_lossy().to_string(),
                    });
            plugin_options.filesystem_scan_path = Some(workspace_path.join("plugins"));
            registry.load_with_options(plugin_options).await?;
            Arc::new(registry)
        };

        let workspace_filesystem = if let Some(fs) = self.workspace_filesystem {
            fs
        } else {
            let object_store_config =
                filesystem_config
                    .clone()
                    .unwrap_or(ObjectStorageConfig::FileSystem {
                        base_path: workspace_path.to_string_lossy().to_string(),
                    });

            let fs_config = distri_filesystem::FileSystemConfig {
                object_store: object_store_config,
                root_prefix: None,
            };

            Arc::new(distri_filesystem::create_file_system(fs_config).await?)
        };

        // Always scope session storage under a stable prefix; paths under here will be
        // threads/{thread}/tasks/{task}/...
        let session_prefix = ".distri/session_storage";
        let session_filesystem = Arc::new(workspace_filesystem.scoped(Some(session_prefix))?);
        let session_root_prefix = Some(session_prefix.to_string());

        let browser_config = Arc::new(RwLock::new(browser_config));
        let browser_sessions = self
            .browser_sessions
            .unwrap_or_else(|| Arc::new(BrowserSessions::new(browser_config.clone())));

        let provider_registry = Arc::new(ProviderRegistry::new());
        if let Err(e) = provider_registry.load_default_providers().await {
            tracing::warn!("Failed to load default OAuth providers: {}", e);
        }
        let tool_auth_handler = Arc::new(OAuthHandler::with_provider_registry(
            stores.tool_auth_store.clone(),
            provider_registry,
            ProviderRegistry::get_callback_url(),
        ));

        // Initialize prompt registry with defaults only (no auto-discovery)
        let prompt_registry = if let Some(registry) = self.prompt_registry {
            registry
        } else {
            Arc::new(PromptRegistry::with_defaults().await.map_err(|e| {
                anyhow::anyhow!("Failed to create prompt registry with defaults: {}", e)
            })?)
        };
        let (default_model_settings, default_analysis_model_settings) = {
            let cfg_guard = configuration.read().await;
            let default_model_settings = self
                .default_model_settings
                .clone()
                .or_else(|| cfg_guard.model_settings.clone())
                .unwrap_or_else(ModelSettings::default);
            let default_analysis_model_settings = self
                .default_analysis_model_settings
                .clone()
                .or_else(|| cfg_guard.analysis_model_settings.clone())
                .unwrap_or_else(|| default_model_settings.clone());
            (
                Arc::new(RwLock::new(default_model_settings)),
                Arc::new(RwLock::new(default_analysis_model_settings)),
            )
        };

        let browser_sessions_clone = browser_sessions.clone();

        let orcheshtrator = AgentOrchestrator {
            tool_auth_handler,
            mcp_registry: registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            workspace_filesystem,
            session_filesystem,
            session_root_prefix,
            browser_config,
            browser_sessions: browser_sessions_clone,
            additional_tools: Arc::new(RwLock::new(self.additional_tools.unwrap_or_default())),
            plugin_registry: plugin_registry.clone(),
            plugin_tools: Arc::new(RwLock::new(self.plugin_tools.unwrap_or_default())),
            workspace_path,
            prompt_registry,
            store_config,
            stores,
            configuration,
            default_model_settings,
            default_analysis_model_settings,
            hooks: hooks.clone(),
            inline_hooks: Arc::new(dashmap::DashMap::new()),
            hook_registry: HookRegistry::new(),
        };

        let orch = Arc::new(orcheshtrator.clone());
        // Set orchestrator on the existing registry (which already has plugins loaded)
        plugin_registry.set_orchestrator(orch.clone() as Arc<dyn OrchestratorTrait>);

        browser_sessions.set_orchestrator(orch.clone() as Arc<dyn OrchestratorTrait>);
        Ok(orcheshtrator)
    }
}

impl AgentOrchestrator {
    fn merge_model_settings(
        base: &ModelSettings,
        agent: &ModelSettings,
        sentinel: &ModelSettings,
    ) -> ModelSettings {
        let provider = if std::mem::discriminant(&agent.provider)
            != std::mem::discriminant(&sentinel.provider)
        {
            agent.provider.clone()
        } else {
            base.provider.clone()
        };

        ModelSettings {
            model: if agent.model != sentinel.model {
                agent.model.clone()
            } else {
                base.model.clone()
            },
            temperature: if (agent.temperature - sentinel.temperature).abs() > f32::EPSILON {
                agent.temperature
            } else {
                base.temperature
            },
            max_tokens: if agent.max_tokens != sentinel.max_tokens {
                agent.max_tokens
            } else {
                base.max_tokens
            },
            context_size: if agent.context_size != sentinel.context_size {
                agent.context_size
            } else {
                base.context_size
            },
            top_p: if (agent.top_p - sentinel.top_p).abs() > f32::EPSILON {
                agent.top_p
            } else {
                base.top_p
            },
            frequency_penalty: if (agent.frequency_penalty - sentinel.frequency_penalty).abs()
                > f32::EPSILON
            {
                agent.frequency_penalty
            } else {
                base.frequency_penalty
            },
            presence_penalty: if (agent.presence_penalty - sentinel.presence_penalty).abs()
                > f32::EPSILON
            {
                agent.presence_penalty
            } else {
                base.presence_penalty
            },
            provider,
            parameters: if agent.parameters.is_some() {
                agent.parameters.clone()
            } else {
                base.parameters.clone()
            },
            response_format: if agent.response_format.is_some() {
                agent.response_format.clone()
            } else {
                base.response_format.clone()
            },
        }
    }

    pub fn cleanup(&self) {
        self.plugin_registry.cleanup();
    }

    pub async fn get_configuration(&self) -> DistriServerConfig {
        self.configuration.read().await.clone()
    }

    pub fn configuration_handle(&self) -> Arc<RwLock<DistriServerConfig>> {
        self.configuration.clone()
    }

    pub async fn update_configuration(&self, configuration: DistriServerConfig) {
        {
            let mut guard = self.configuration.write().await;
            *guard = configuration.clone();
        }
        if let Some(model_settings) = configuration.model_settings.clone() {
            let mut guard = self.default_model_settings.write().await;
            *guard = model_settings.clone();
        }
        if let Some(analysis_settings) = configuration
            .analysis_model_settings
            .clone()
            .or(configuration.model_settings)
        {
            let mut guard = self.default_analysis_model_settings.write().await;
            *guard = analysis_settings;
        }
    }

    pub async fn get_default_model_settings(&self) -> ModelSettings {
        self.default_model_settings.read().await.clone()
    }

    pub async fn get_default_analysis_model_settings(&self) -> ModelSettings {
        self.default_analysis_model_settings.read().await.clone()
    }

    /// Create ephemeral session stores for a single thread execution
    /// This creates new, isolated stores that will be automatically cleaned up when dropped
    pub async fn create_ephemeral_session_stores(
        &self,
    ) -> anyhow::Result<distri_stores::SessionStores> {
        distri_stores::create_ephemeral_session_stores().await
    }

    /// Check if ephemeral stores are enabled
    pub fn is_ephemeral(&self) -> bool {
        self.store_config.session.ephemeral
    }

    /// Create fresh ephemeral session stores and update context if needed
    async fn prepare_execution_context(
        &self,
        context: Arc<ExecutorContext>,
    ) -> Result<Arc<ExecutorContext>, AgentError> {
        let mut ctx = Arc::try_unwrap(context).unwrap_or_else(|arc| (*arc).clone());

        if let Some(prefix) = self.session_root_prefix.clone() {
            let metadata = ctx
                .tool_metadata
                .get_or_insert_with(std::collections::HashMap::new);
            metadata
                .entry("filesystem_root_prefix".to_string())
                .or_insert(serde_json::Value::String(prefix));
        }

        if self.is_ephemeral() && ctx.stores.is_none() {
            let execution_stores = distri_stores::create_ephemeral_execution_stores(&self.stores)
                .await
                .map_err(|e| {
                    AgentError::Other(format!("Failed to create ephemeral stores: {}", e))
                })?;
            ctx.stores = Some(execution_stores);
        }

        Ok(Arc::new(ctx))
    }

    pub async fn register_mcp_server(&self, name: String, server: ServerMetadataWrapper) {
        let registry = self.mcp_registry.clone();
        registry.write().await.register(name, server);
    }

    pub async fn register_agent_definition(
        &self,
        definition: crate::types::StandardDefinition,
    ) -> anyhow::Result<()> {
        tracing::debug!("🤖 Registering agent definition: {}", definition.name);
        // Register agent's custom partials with the prompt registry
        for (name, path) in &definition.partials {
            match self.register_prompt_partial_file(name.clone(), path).await {
                Ok(()) => {
                    tracing::debug!("✅ Registered partial '{}' from '{}'", name, path);
                }
                Err(e) => {
                    tracing::warn!(
                        "⚠️  Failed to register partial '{}' from '{}': {}",
                        name,
                        path,
                        e
                    );
                    // Continue with other partials instead of failing completely
                }
            }
        }

        let agent_config =
            distri_types::configuration::AgentConfig::StandardAgent(definition.clone());
        self.stores
            .agent_store
            .register(agent_config)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub async fn register_tool(&self, agent_id: &str, tool: Arc<dyn Tool>) {
        let mut additional_tools = self.additional_tools.write().await;
        additional_tools
            .entry(agent_id.to_string())
            .or_insert(vec![])
            .push(tool);
    }

    /// Get the prompt registry for registering/accessing prompt templates
    pub fn get_prompt_registry(&self) -> Arc<PromptRegistry> {
        self.prompt_registry.clone()
    }

    /// Register a prompt template dynamically
    pub async fn register_prompt_template(
        &self,
        name: String,
        content: String,
        description: Option<String>,
        version: Option<String>,
    ) -> Result<(), AgentError> {
        self.prompt_registry
            .register_template_string(name, content, description, version)
            .await
    }

    /// Get a prompt template by name
    pub async fn get_prompt_template(
        &self,
        name: &str,
    ) -> Option<crate::agent::prompt_registry::PromptTemplate> {
        self.prompt_registry.get_template(name).await
    }

    /// Register a prompt partial dynamically
    pub async fn register_prompt_partial(
        &self,
        name: String,
        content: String,
    ) -> Result<(), AgentError> {
        self.prompt_registry.register_partial(name, content).await
    }

    /// Register a partial from a file
    pub async fn register_prompt_partial_file<P: AsRef<std::path::Path>>(
        &self,
        name: String,
        file_path: P,
    ) -> Result<(), AgentError> {
        self.prompt_registry
            .register_partial_file(name, file_path)
            .await
    }

    /// Register distri tools - load plugin tools once and store them in the orchestrator
    pub async fn register_distri_tools(&self) -> anyhow::Result<()> {
        let plugin_records = self
            .plugin_registry
            .list_plugin_records()
            .await?
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>();
        let total_tools = plugin_records
            .iter()
            .map(|record| record.artifact.tools.len())
            .sum::<usize>();
        tracing::debug!(
            "Distri registry ready with {} packages and {} tools",
            plugin_records.len(),
            total_tools
        );

        let plugin_tools = self.plugin_registry.get_plugin_tools().await?;

        // Store the loaded plugin tools in the orchestrator
        {
            let mut plugin_tools_guard = self.plugin_tools.write().await;
            *plugin_tools_guard = plugin_tools;
        }

        tracing::debug!("DAP tools loaded and cached in orchestrator");

        Ok(())
    }

    /// Register default agents
    pub async fn register_distri_agents(&self) -> anyhow::Result<()> {
        let plugin_records = self
            .plugin_registry
            .list_plugin_records()
            .await?
            .into_iter()
            .map(Arc::new)
            .collect::<Vec<_>>();
        let total_agents = plugin_records
            .iter()
            .map(|record| record.artifact.agents.len())
            .sum::<usize>();

        // self.stores.agent_store.clear().await?;

        tracing::debug!(
            "Registering {} distri agents from {} packages",
            total_agents,
            plugin_records.len()
        );

        // 1. First register plugin agents and workflows with package prefix (lowest priority)
        for record in &plugin_records {
            for agent in &record.artifact.agents {
                let agent_name = namespace_plugin_item(&agent.package_name, &agent.name);
                let mut agent_config = agent.agent_config.clone();
                match &mut agent_config {
                    distri_types::configuration::AgentConfig::StandardAgent(def) => {
                        def.name = agent_name.clone()
                    }
                    distri_types::configuration::AgentConfig::SequentialWorkflowAgent(def) => {
                        def.name = agent_name.clone()
                    }
                    distri_types::configuration::AgentConfig::DagWorkflowAgent(def) => {
                        def.name = agent_name.clone()
                    }
                    distri_types::configuration::AgentConfig::CustomAgent(def) => {
                        def.name = agent_name.clone();
                        def.package = Some(agent.package_name.clone());
                    }
                }

                tracing::debug!("🤖 Registering plugin agent: {}", agent_name);
                self.stores.agent_store.register(agent_config).await?;
            }

            for workflow in &record.artifact.workflows {
                let agent_name = namespace_plugin_item(&workflow.package_name, &workflow.name);
                let examples = workflow
                    .examples
                    .iter()
                    .filter_map(|value| {
                        serde_json::from_value::<CustomAgentExample>(value.clone()).ok()
                    })
                    .collect();

                let custom_agent_def = CustomAgentDefinition {
                    name: agent_name.clone(),
                    description: workflow.description.clone(),
                    script_path: format!("workflows/{}.ts", workflow.name),
                    package: Some(workflow.package_name.clone()),
                    parameters: workflow.parameters.clone(),
                    examples,
                    working_directory: None,
                };
                let agent_config =
                    distri_types::configuration::AgentConfig::CustomAgent(custom_agent_def);

                tracing::debug!("🤖 Registering workflow as agent: {}", agent_name);
                self.stores.agent_store.register(agent_config).await?;
            }
        }

        tracing::debug!("Successfully registered {} plugin agents", total_agents);
        Ok(())
    }
    pub async fn get_agent_tools(
        &self,
        definition: &crate::types::StandardDefinition,
        external_tools: &[Arc<dyn Tool>],
    ) -> Result<Vec<Arc<dyn Tool>>, AgentError> {
        // Use new tools configuration if available, fallback to old mcp_servers
        let tools_config = definition.tools.clone().unwrap_or(ToolsConfig::default());

        // Get cached plugin tools from orchestrator
        let plugin_tools = {
            let plugin_tools_guard = self.plugin_tools.read().await;
            plugin_tools_guard.clone()
        };

        let mut tools = crate::tools::resolve_tools_config(
            &tools_config,
            self.mcp_registry.clone(),
            plugin_tools,
            self.workspace_filesystem.clone(),
            self.session_filesystem.clone(),
            definition.file_system.include_server_tools(),
            external_tools,
        )
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        // Get agent-specific additional tools (including DAP tools for this agent)
        let additional_tools = {
            let tools = self.additional_tools.read().await;
            tools.get(&definition.name).unwrap_or(&vec![]).clone()
        };
        tools.extend(additional_tools.iter().cloned());

        // Add TodosDelegateTool if todos are enabled for this agent
        if definition.is_todos_enabled() {
            let todos_tool = Arc::new(TodosTool) as Arc<dyn Tool>;
            let has_todos = tools.iter().any(|t| t.get_name() == todos_tool.get_name());
            if !has_todos {
                tools.push(todos_tool);
            }
        }

        if definition.sub_agents.is_empty() {
            tools.retain(|t| t.get_name() != "transfer_to_agent");
        } else {
            // Add AgentTool instances for each sub-agent
            for sub_agent_name in &definition.sub_agents {
                tools.push(Arc::new(crate::tools::AgentTool::new(
                    sub_agent_name.clone(),
                )));
            }
        }

        Ok(tools)
    }

    /// Create an agent instance from a config using the factory
    pub async fn create_agent_from_config(
        &self,
        config: distri_types::configuration::AgentConfig,
        context: Arc<ExecutorContext>,
    ) -> Result<Box<dyn BaseAgent>, AgentError> {
        match config {
            distri_types::configuration::AgentConfig::StandardAgent(definition) => {
                let external_tools = {
                    match context.dynamic_tools.clone() {
                        Some(tools) => tools.read().await.clone(),
                        None => vec![],
                    }
                };
                let tools = self.get_agent_tools(&definition, &external_tools).await?;
                context.extend_tools(tools).await;
                let tools = context.get_tools().await;

                let hook_impl: Arc<dyn crate::agent::types::AgentHooks> = {
                    let hook_map = self.hooks.read().await;
                    let mut hooks = Vec::new();
                    for hook_name in &definition.hooks {
                        if let Some(hook) = hook_map.get(hook_name) {
                            hooks.push(hook.clone());
                        } else {
                            tracing::warn!(
                                "Hook '{}' not found for agent '{}'",
                                hook_name,
                                definition.name
                            );
                        }
                    }

                    match hooks.len() {
                        0 => Arc::new(crate::agent::standard::DefaultHooks),
                        1 => hooks.remove(0),
                        _ => Arc::new(crate::agent::hooks::CombinedHooks::new(hooks)),
                    }
                };

                let agent = crate::agent::StandardAgent::new(
                    definition,
                    tools,
                    self.stores.external_tool_calls_store.clone(),
                    hook_impl,
                )
                .await?;
                Ok(Box::new(agent))
            }
            distri_types::configuration::AgentConfig::SequentialWorkflowAgent(definition) => {
                let tools = self.get_all_available_tools().await?;
                let agent = crate::agent::WorkflowAgent::new_sequential(definition, tools);
                Ok(Box::new(agent))
            }
            distri_types::configuration::AgentConfig::DagWorkflowAgent(definition) => {
                let tools = self.get_all_available_tools().await?;
                let agent = crate::agent::WorkflowAgent::new_dag(definition, tools);
                Ok(Box::new(agent))
            }
            distri_types::configuration::AgentConfig::CustomAgent(definition) => {
                let tools = self.get_all_available_tools().await?;
                let agent = crate::agent::WorkflowAgent::new_custom(definition, tools);
                Ok(Box::new(agent))
            }
        }
    }

    /// Update an existing agent with new definition
    pub async fn update_agent_definition(
        &self,
        definition: crate::types::StandardDefinition,
    ) -> anyhow::Result<()> {
        let agent_config =
            distri_types::configuration::AgentConfig::StandardAgent(definition.clone());
        self.stores
            .agent_store
            .update(agent_config)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        tracing::debug!("AgentCoordinator run loop started");

        while let Some(msg) = self.coordinator_rx.lock().await.recv().await {
            tracing::debug!("AgentCoordinator received a message: {:?}", msg);
            match msg {
                CoordinatorMessage::ExecuteStream {
                    agent_id,
                    message,
                    context,
                } => {
                    tracing::debug!(
                        "Handling ExecuteStream for agent: {} with message: {:?}",
                        agent_id,
                        message
                    );
                    let this = self.clone();
                    tokio::spawn(async move {
                        let result = async {
                            this.call_agent_stream(&agent_id, message, context, None)
                                .await
                        }
                        .await;

                        if let Err(e) = result {
                            tracing::error!("Error in Coordinator:ExecuteStream: {}", e);
                        }
                    });
                }
                CoordinatorMessage::HandoverAgent {
                    from_agent,
                    to_agent,
                    reason,
                    context,
                } => {
                    tracing::debug!(
                        "Handling agent handover from {} to {}",
                        from_agent,
                        to_agent
                    );

                    // Emit the AgentHandover event if event_tx is available
                    context
                        .emit(AgentEventType::AgentHandover {
                            from_agent: from_agent.clone(),
                            to_agent: to_agent.clone(),
                            reason,
                        })
                        .await;
                }
            }
        }
        tracing::info!("AgentCoordinator run loop exiting");
        Ok(())
    }

    async fn call_agent(
        &self,
        agent_id: &str,
        message: Message,
        context: Arc<ExecutorContext>,
        definition_overrides: Option<DefinitionOverrides>,
    ) -> Result<InvokeResult, AgentError> {
        // Get agent definition and create agent instance
        let mut agent_config = self
            .get_agent(agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

        // Apply definition overrides if provided (only for StandardAgent)
        self.apply_agent_overrides(&mut agent_config, definition_overrides)
            .await;

        let agent = self
            .create_agent_from_config(agent_config, context.clone())
            .await?;
        let (tx, mut rx) = tokio::sync::mpsc::channel(10000);
        let context_with_tx = context.clone_with_tx(tx);
        let handle = tokio::spawn(async move { while let Some(_) = rx.recv().await {} });
        let result = agent
            .invoke_stream(message, Arc::new(context_with_tx))
            .await?;
        let _ = handle.await;
        Ok(result)
    }

    async fn call_agent_stream(
        &self,
        agent_id: &str,
        message: Message,
        context: Arc<ExecutorContext>,
        definition_overrides: Option<DefinitionOverrides>,
    ) -> Result<InvokeResult, AgentError> {
        tracing::debug!("call_agent_stream called with agent_id: {}", agent_id);
        // Get agent config and create agent instance
        let mut agent_config = self
            .get_agent(agent_id)
            .await
            .ok_or_else(|| AgentError::NotFound(format!("Agent {} not found", agent_id)))?;

        // Apply definition overrides if provided (only for StandardAgent)
        self.apply_agent_overrides(&mut agent_config, definition_overrides)
            .await;

        // Check if todos are enabled for this agent and initialize shared_todos if needed
        if let distri_types::configuration::AgentConfig::StandardAgent(definition) = &agent_config {
            if definition.should_use_browser() {
                tracing::debug!("🌐 Browser enabled for agent: {}", agent_id);
                // No in-process browser initialization; sessions are handled via browsr-client.
            }
        }

        tracing::debug!(
            "Creating agent from config: {:?}",
            std::mem::discriminant(&agent_config)
        );
        let agent: Box<dyn BaseAgent> = self
            .create_agent_from_config(agent_config, context.clone())
            .await?;
        tracing::debug!("Created agent type: {}", agent.get_name());

        agent.invoke_stream(message, context).await
    }

    pub async fn run_inline_agent(
        &self,
        agent_config: distri_types::configuration::AgentConfig,
        task: &str,
        context: Arc<ExecutorContext>,
    ) -> Result<InvokeResult, AgentError> {
        let agent: Box<dyn BaseAgent> = self
            .create_agent_from_config(agent_config.clone(), context.clone())
            .await?;

        let message = Message {
            id: uuid::Uuid::new_v4().to_string(),
            name: None,
            parts: vec![Part::Text(task.to_string())],
            role: distri_types::MessageRole::User,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        agent.invoke_stream(message, context).await
    }

    // Thread management methods
    pub async fn create_thread(&self, request: CreateThreadRequest) -> Result<Thread, AgentError> {
        self.stores
            .thread_store
            .create_thread(request)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn get_thread(&self, thread_id: &str) -> Result<Option<Thread>, AgentError> {
        self.stores
            .thread_store
            .get_thread(thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> Result<Thread, AgentError> {
        self.stores
            .thread_store
            .update_thread(thread_id, request)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn delete_thread(&self, thread_id: &str) -> Result<(), AgentError> {
        self.stores
            .thread_store
            .delete_thread(thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn list_threads(
        &self,
        user_id: Option<&str>,
        limit: Option<u32>,
        offset: Option<u32>,
        filter: Option<&serde_json::Value>,
    ) -> Result<Vec<ThreadSummary>, AgentError> {
        self.stores
            .thread_store
            .list_threads(user_id, limit, offset, filter)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    /// Ensures a thread exists for the given agent and thread_id, creating it if necessary.
    /// Optionally takes an initial message for thread creation.
    pub async fn ensure_thread_exists_with_store(
        &self,
        agent_id: &str,
        thread_id: Option<String>,
        title: Option<&str>,
        attributes: Option<serde_json::Value>,
        thread_store: &Arc<dyn ThreadStore>,
    ) -> Result<Thread, AgentError> {
        // For ephemeral mode, skip the get_thread check and always create new thread
        // This avoids "failed to load thread" errors with fresh ephemeral databases
        let thread = if self.is_ephemeral() {
            None
        } else {
            match &thread_id {
                Some(thread_id) => thread_store
                    .get_thread(thread_id)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))?,
                None => None,
            }
        };

        match thread {
            Some(existing) => {
                if let Some(attrs) = attributes {
                    let update_req = crate::types::UpdateThreadRequest {
                        title: None,
                        metadata: None,
                        attributes: Some(attrs),
                    };
                    let updated = thread_store
                        .update_thread(&existing.id, update_req)
                        .await
                        .map_err(|e| AgentError::Session(e.to_string()))?;
                    Ok(updated)
                } else {
                    Ok(existing)
                }
            }
            None => {
                let create_request = crate::types::CreateThreadRequest {
                    agent_id: agent_id.to_string(),
                    title: title.map(String::from),
                    thread_id,
                    attributes,
                };
                thread_store
                    .create_thread(create_request)
                    .await
                    .map_err(|e| AgentError::Session(e.to_string()))
            }
        }
    }

    pub async fn ensure_thread_exists(
        &self,
        agent_id: &str,
        thread_id: Option<String>,
        title: Option<&str>,
        attributes: Option<serde_json::Value>,
    ) -> Result<Thread, AgentError> {
        self.ensure_thread_exists_with_store(
            agent_id,
            thread_id,
            title,
            attributes,
            &self.stores.thread_store,
        )
        .await
    }

    pub async fn execute(
        &self,
        agent_name: &str,
        message: Message,
        context: Arc<ExecutorContext>,
        definition_overrides: Option<DefinitionOverrides>,
    ) -> Result<InvokeResult, AgentError> {
        // Prepare context with ephemeral stores if needed
        let context = self.prepare_execution_context(context).await?;

        // Use context stores if provided, otherwise use orchestrator stores
        let stores = context.stores.as_ref().unwrap_or(&self.stores);

        self.ensure_thread_exists_with_store(
            &agent_name,
            Some(context.thread_id.clone()),
            message.as_text().as_deref(),
            context
                .additional_attributes
                .clone()
                .map(|a| a.thread)
                .flatten(),
            &stores.thread_store,
        )
        .await?;

        stores
            .task_store
            .get_or_create_task(&context.thread_id, &context.task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        if let Some(parent) = context.parent_task_id.as_deref() {
            let _ = stores
                .task_store
                .update_parent_task(&context.task_id, Some(parent))
                .await;
        }

        self.validate_user_message(&message)?;

        self.call_agent(agent_name, message, context, definition_overrides)
            .await
    }

    pub async fn execute_stream(
        &self,
        agent_name: &str,
        message: Message,
        context: Arc<ExecutorContext>,
        definition_overrides: Option<DefinitionOverrides>,
    ) -> Result<InvokeResult, AgentError> {
        // Prepare context with ephemeral stores if needed
        let context = self.prepare_execution_context(context).await?;

        // Use context stores if provided, otherwise use orchestrator stores
        let stores = context.stores.as_ref().unwrap_or(&self.stores);

        self.ensure_thread_exists_with_store(
            &agent_name,
            Some(context.thread_id.clone()),
            message.as_text().as_deref(),
            context
                .additional_attributes
                .as_ref()
                .map(|a| a.thread.clone())
                .flatten(),
            &stores.thread_store,
        )
        .await?;

        stores
            .task_store
            .get_or_create_task(&context.thread_id, &context.task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        if let Some(parent) = context.parent_task_id.as_deref() {
            let _ = stores
                .task_store
                .update_parent_task(&context.task_id, Some(parent))
                .await;
        }

        self.validate_user_message(&message)?;

        let res = self
            .call_agent_stream(agent_name, message, context.clone(), definition_overrides)
            .await?;

        Ok(res)
    }

    pub fn validate_user_message(&self, message: &Message) -> Result<(), AgentError> {
        tracing::debug!("Validating message: {:#?}", message);
        if message.parts.is_empty() {
            return Err(AgentError::Validation("Message has no parts".to_string()));
        }

        // Extract tool responses from the message
        let valid_parts = message
            .parts
            .iter()
            .any(|part| matches!(part, Part::ToolResult(_) | Part::Text(_)));

        if !valid_parts {
            return Err(AgentError::Validation(
                "Message has to contain either text or tool parts".to_string(),
            ));
        }

        Ok(())
    }

    pub async fn list_agents(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (
        Vec<distri_types::configuration::AgentConfig>,
        Option<String>,
    ) {
        let (agents, next_cursor) = self.stores.agent_store.list(cursor, limit).await;
        // Default agents are now loaded via DapRegistry
        (agents, next_cursor)
    }

    pub async fn apply_agent_overrides(
        &self,
        agent_config: &mut distri_types::configuration::AgentConfig,
        definition_overrides: Option<DefinitionOverrides>,
    ) {
        if let distri_types::configuration::AgentConfig::StandardAgent(ref mut definition) =
            agent_config
        {
            // Start from orchestrator defaults, then overlay agent-specific settings so agent wins.
            let default_model_settings = self.get_default_model_settings().await;
            let sentinel = ModelSettings::default();
            let agent_model = definition.model_settings.clone();
            definition.model_settings =
                Self::merge_model_settings(&default_model_settings, &agent_model, &sentinel);

            let default_analysis_settings = self.get_default_analysis_model_settings().await;
            definition.analysis_model_settings = definition
                .analysis_model_settings
                .clone()
                .map(|agent_analysis| {
                    Self::merge_model_settings(
                        &default_analysis_settings,
                        &agent_analysis,
                        &sentinel,
                    )
                })
                .or(Some(default_analysis_settings));
            tracing::debug!("Applying definition overrides: {:?}", definition_overrides);
            if let Some(overrides) = definition_overrides {
                definition.apply_overrides(overrides);
            }
        }
    }

    /// Search for agent in DAP registry
    pub async fn get_agent(&self, name: &str) -> Option<distri_types::configuration::AgentConfig> {
        // Then try exact match in store (includes full package/agent names)
        if let Some(agent) = self.stores.agent_store.get(name).await {
            // Note: todos support is now handled via TodosDelegateTool in get_agent_tools
            // rather than adding a subagent
            return Some(agent);
        }

        if is_namespaced_plugin_id(name) {
            return None;
        }

        let mut cursor = None;
        loop {
            let (agents, next_cursor) = self.stores.agent_store.list(cursor.clone(), None).await;
            if let Some(agent) = agents
                .into_iter()
                .find(|agent| Self::agent_matches_simple_name(agent, name))
            {
                return Some(agent);
            }

            if let Some(next) = next_cursor {
                cursor = Some(next);
            } else {
                break;
            }
        }

        None
    }

    fn agent_matches_simple_name(
        agent: &distri_types::configuration::AgentConfig,
        target: &str,
    ) -> bool {
        let full_name = agent.get_name();
        if full_name == target {
            return true;
        }

        if let Some((_, simple)) = split_namespaced_plugin_id(&full_name) {
            if simple == target {
                return true;
            }
        }

        if let Some((target_package, target_simple)) = split_namespaced_plugin_id(target) {
            return match agent {
                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                    def.package_name
                        .as_deref()
                        .map(|pkg| pkg == target_package)
                        .unwrap_or(false)
                        && def.name == target_simple
                }
                distri_types::configuration::AgentConfig::CustomAgent(def) => {
                    def.package
                        .as_deref()
                        .map(|pkg| pkg == target_package)
                        .unwrap_or(false)
                        && def.name == target_simple
                }
                _ => false,
            };
        }

        false
    }

    /// Whether a shared browser session is currently active.
    pub async fn browser_session_active(&self) -> bool {
        !self.browser_sessions.list().is_empty()
    }

    pub async fn create_browser_session(
        &self,
        requested_name: Option<String>,
    ) -> Result<(String, Arc<Mutex<()>>), String> {
        self.browser_sessions.create(requested_name).await
    }

    pub async fn ensure_browser_session(
        &self,
        requested: Option<String>,
    ) -> Result<(String, Arc<Mutex<()>>), String> {
        self.browser_sessions.ensure(requested).await
    }

    pub fn list_browser_sessions(&self) -> Vec<String> {
        self.browser_sessions.list()
    }

    pub fn stop_browser_session(&self, id: &str) -> bool {
        self.browser_sessions.stop(id)
    }

    /// Update task status
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: crate::types::TaskStatus,
    ) -> Result<(), anyhow::Error> {
        // Clone the Arc to keep stores alive during the async operation
        let task_store = self.stores.task_store.clone();
        task_store.update_task_status(task_id, status).await
    }

    /// Get all available tools (MCP tools + plugin tools) - standardized method for tool discovery
    /// Uses the existing resolve_tools_config with a "get all tools" configuration
    /// Returns tools with both simple names and package.tool_name format for namespace support
    pub async fn get_all_available_tools(
        &self,
    ) -> Result<Vec<Arc<dyn crate::tools::Tool>>, AgentError> {
        use crate::types::{McpToolConfig, ToolsConfig};
        use std::collections::HashMap;

        let builtin_tools = crate::tools::get_builtin_tools(
            self.workspace_filesystem.clone(),
            self.session_filesystem.clone(),
            true,
        );
        // Create a ToolsConfig that includes all available tools
        let mut all_tools_config = ToolsConfig {
            builtin: vec![], // Don't include builtin tools by default for /toolcall
            packages: HashMap::new(),
            mcp: Vec::new(),
            external: None,
        };

        // Add all MCP servers with wildcard to get all their tools
        {
            let registry = self.mcp_registry.clone();
            let servers = registry.read().await;
            for server_name in servers.servers.keys() {
                all_tools_config.mcp.push(McpToolConfig {
                    server: server_name.clone(),
                    include: vec!["*".to_string()], // Get all tools from this server
                    exclude: vec![],                // Don't exclude anything
                });
            }
        }

        // Add all plugin packages with wildcard to get all their tools
        let plugin_tools = {
            let plugin_tools_guard = self.plugin_tools.read().await;
            plugin_tools_guard.clone()
        };

        for package_name in plugin_tools.keys() {
            all_tools_config.packages.insert(
                package_name.clone(),
                vec!["*".to_string()], // Get all tools from this package
            );
        }

        // Use the standardized resolve_tools_config method
        let mut tools = crate::tools::resolve_tools_config(
            &all_tools_config,
            self.mcp_registry.clone(),
            plugin_tools,
            self.workspace_filesystem.clone(),
            self.session_filesystem.clone(),
            true,
            &[],
        )
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        tools.extend(builtin_tools);

        Ok(tools)
    }

    /// Complete an external tool execution
    pub async fn complete_tool(
        &self,
        tool_call_id: &str,
        tool_response: distri_types::ToolResponse,
    ) -> Result<(), String> {
        self.stores
            .external_tool_calls_store
            .complete_external_tool_call(tool_call_id, tool_response)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn complete_inline_hook(
        &self,
        hook_id: &str,
        mutation: HookMutation,
    ) -> Result<(), String> {
        if let Some((_, tx)) = self.inline_hooks.remove(hook_id) {
            let _ = tx.send(mutation);
            Ok(())
        } else {
            Err(format!("Inline hook {} not found", hook_id))
        }
    }

    /// Find a tool by name, supporting simple names and plugin::tool names
    pub fn find_tool_by_name(
        &self,
        tools: &[Arc<dyn crate::tools::Tool>],
        tool_name: &str,
    ) -> Option<Arc<dyn crate::tools::Tool>> {
        crate::agent::tool_lookup::find_tool_by_name(tools, tool_name)
    }

    pub async fn call_tool_with_context(
        &self,
        tool_call: &ToolCall,
        context: Arc<ExecutorContext>,
    ) -> anyhow::Result<serde_json::Value> {
        let tools = self
            .get_all_available_tools()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get available tools: {}", e))?;

        let tool = self
            .find_tool_by_name(&tools, &tool_call.tool_name)
            .ok_or_else(|| anyhow::anyhow!("Tool {} not found", tool_call.tool_name))?;

        let result = if tool.needs_executor_context() {
            crate::tools::execute_tool_with_executor_context(
                tool.as_ref(),
                tool_call.clone(),
                context,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute tool: {}", e))?
        } else {
            let tool_context = crate::tools::context::to_tool_context(&context);
            tool.execute(tool_call.clone(), Arc::new(tool_context))
                .await?
        };

        // Convert Vec<Part> back to Value for API compatibility
        let value = if result.len() == 1 {
            match &result[0] {
                distri_types::Part::Data(data) => data.clone(),
                _ => serde_json::json!({"result": result}),
            }
        } else {
            serde_json::json!({"parts": result})
        };

        Ok(value)
    }
}

// Default agents are now loaded from the default_agents directory via DapRegistry

// Implement WorkflowRuntime trait for WorkflowExecutor
#[async_trait::async_trait]
impl OrchestratorTrait for AgentOrchestrator {
    /// Get a session value for a specific session
    async fn get_session_value(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        self.stores
            .session_store
            .get_value(session_id, key)
            .await
            .ok()
            .flatten()
    }

    /// Set a session value for a specific session
    async fn set_session_value(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.stores
            .session_store
            .set_value(session_id, key, &value)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set session value: {}", e))
    }

    /// Call an agent via the orchestrator
    async fn call_agent(
        &self,
        session_id: &str,
        agent_name: &str,
        task: &str,
    ) -> anyhow::Result<String> {
        // Use the orchestrator to call the agent
        let context = ExecutorContext {
            session_id: session_id.to_string(),
            orchestrator: Some(Arc::new(self.clone())),
            stores: Some(self.stores.clone()),
            ..Default::default()
        };
        let stream = self
            .execute(
                agent_name,
                Message::user(task.to_string(), None),
                Arc::new(context.clone()),
                None,
            )
            .await?;

        Ok(stream.content.unwrap_or_default())
    }

    async fn call_tool(
        &self,
        session_id: &str,
        user_id: &str,
        tool_call: &ToolCall,
    ) -> anyhow::Result<serde_json::Value> {
        let context = ExecutorContext {
            session_id: session_id.to_string(),
            user_id: user_id.to_string(),
            agent_id: format!("tool:{}", tool_call.tool_name),
            orchestrator: Some(Arc::new(self.clone())),
            stores: Some(self.stores.clone()),
            ..Default::default()
        };

        self.call_tool_with_context(tool_call, Arc::new(context))
            .await
    }

    async fn llm_execute(
        &self,
        llm_def: LlmDefinition,
        llm_context: LLmContext,
    ) -> Result<serde_json::Value, anyhow::Error> {
        let mut ctx = ExecutorContext::default();

        if let Some(thread_id) = llm_context.thread_id {
            ctx.thread_id = thread_id;
        }
        if let Some(task_id) = llm_context.task_id {
            ctx.task_id = task_id;
        }
        if let Some(run_id) = llm_context.run_id {
            ctx.run_id = run_id;
        }

        ctx.agent_id = llm_def.name.clone();
        let messages = if llm_context.messages.is_empty() {
            vec![Message::user("".to_string(), None)]
        } else {
            llm_context.messages
        };

        let executor = LLMExecutor::new(
            llm_def,
            vec![],
            Arc::new(ctx),
            None,
            llm_context.label.clone(),
        );
        let response = executor.execute(&messages).await?;
        Ok(parse_structured_output(&response.content))
    }
}

fn parse_structured_output(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}
