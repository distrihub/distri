use crate::{
    agent::{
        prompt_registry::PromptRegistry, todos::TodosTool, AgentEventType, BaseAgent,
        CoordinatorMessage, InvokeResult,
    },
    servers::registry::McpServerRegistry,
    tools::{FinalTool, Tool},
    types::{CreateThreadRequest, Message, Thread, UpdateThreadRequest},
    AgentError, HookRegistry,
};

use super::ExecutorContext;
use crate::agent::hooks::inline::InlineHook;
use distri_filesystem::FileSystem;
use distri_stores::{initialize_stores, InitializedStores};
pub use distri_stores::{AgentStore, ThreadStore};
use distri_types::configuration::AgentConfig;
use distri_types::stores::{PromptTemplateStore, SecretStore};
use distri_types::{
    browser::BrowsrClientConfig,
    configuration::{DistriServerConfig, StoreConfig},
    HookMutation,
};
use distri_types::{
    configuration::{DefinitionOverrides, ObjectStorageConfig},
    LLmContext, OrchestratorTrait,
};
use distri_types::{
    LlmDefinition, ModelSettings, Part, ServerMetadataWrapper, ToolCall, ToolsConfig,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};

pub const SKILL_STORAGE_ROOT: &str = "storage/skills";

#[derive(Clone)]
pub struct AgentOrchestrator {
    pub mcp_registry: Arc<RwLock<McpServerRegistry>>,
    pub coordinator_rx: Arc<Mutex<mpsc::Receiver<CoordinatorMessage>>>,
    pub coordinator_tx: mpsc::Sender<CoordinatorMessage>,
    pub workspace_filesystem: Arc<FileSystem>,
    pub session_filesystem: Arc<FileSystem>,
    pub session_root_prefix: Option<String>,
    pub browser_config: Arc<RwLock<BrowsrClientConfig>>,
    pub additional_tools: Arc<RwLock<HashMap<String, Vec<Arc<dyn Tool>>>>>,
    pub workspace_path: std::path::PathBuf,
    pub prompt_registry: Arc<PromptRegistry>,
    /// Store configuration for creating new session stores
    pub store_config: StoreConfig,
    /// All stores - use this instead of individual store fields
    pub stores: InitializedStores,
    pub configuration: Arc<RwLock<DistriServerConfig>>,
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
    user_id: Option<String>,
    registry: Option<Arc<RwLock<McpServerRegistry>>>,
    additional_tools: Option<HashMap<String, Vec<Arc<dyn Tool>>>>,
    workspace_filesystem: Option<Arc<FileSystem>>,
    workspace_path: Option<std::path::PathBuf>,
    browser_config: Option<BrowsrClientConfig>,
    stores: Option<InitializedStores>,
    prompt_registry: Option<Arc<PromptRegistry>>,
    store_config: Option<StoreConfig>,
    configuration: Option<Arc<RwLock<DistriServerConfig>>>,
    hooks: Option<HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>>,
}

impl AgentOrchestratorBuilder {
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

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

    pub fn with_browser_config(mut self, browser_config: BrowsrClientConfig) -> Self {
        self.browser_config = Some(browser_config);
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

        // Initialize prompt registry with defaults only (no auto-discovery)
        // User-specific partials are loaded at render time in formatter.rs
        let prompt_registry = if let Some(registry) = self.prompt_registry {
            registry
        } else {
            Arc::new(PromptRegistry::with_defaults().await.map_err(|e| {
                anyhow::anyhow!("Failed to create prompt registry with defaults: {}", e)
            })?)
        };

        let orchestrator = AgentOrchestrator {
            mcp_registry: registry,
            coordinator_rx: Arc::new(Mutex::new(coordinator_rx)),
            coordinator_tx,
            workspace_filesystem,
            session_filesystem,
            session_root_prefix,
            browser_config,
            additional_tools: Arc::new(RwLock::new(self.additional_tools.unwrap_or_default())),
            workspace_path,
            prompt_registry,
            store_config,
            stores,
            configuration,
            hooks: hooks.clone(),
            inline_hooks: Arc::new(dashmap::DashMap::new()),
            hook_registry: HookRegistry::new(),
        };

        // Sync system prompts to the store
        if let Some(store) = &orchestrator.stores.prompt_template_store {
            let defaults = PromptRegistry::get_default_templates();
            if let Err(e) = store.sync_system_templates(defaults).await {
                tracing::warn!("⚠️  Failed to sync system prompts to store: {}", e);
            }
        }

        Ok(orchestrator)
    }
}

impl AgentOrchestrator {
    /// Merge base model settings with agent-level overrides.
    ///
    /// Individual fields on `agent` are considered "explicitly set" when:
    /// - `model` is non-empty
    /// - `provider` differs from the default (OpenAI)
    /// - `context_size` differs from the default
    /// - Option fields are Some
    pub(crate) fn merge_model_settings(
        base: &ModelSettings,
        agent: &ModelSettings,
    ) -> Result<ModelSettings, AgentError> {
        let default_provider = distri_types::ModelProvider::OpenAI {};
        let provider = if std::mem::discriminant(&agent.inner.provider)
            != std::mem::discriminant(&default_provider)
        {
            agent.inner.provider.clone()
        } else {
            base.inner.provider.clone()
        };

        let model = if !agent.model.is_empty() {
            agent.model.clone()
        } else {
            base.model.clone()
        };
        if model.is_empty() {
            return Err(AgentError::InvalidConfiguration(
                "model not set — configure default_model_settings on the orchestrator or workspace"
                    .to_string(),
            ));
        }

        let default_context_size = 20000u32;
        Ok(ModelSettings {
            model,
            inner: distri_types::ModelSettingsInner {
                temperature: agent.inner.temperature.or(base.inner.temperature),
                max_tokens: agent.inner.max_tokens.or(base.inner.max_tokens),
                context_size: if agent.inner.context_size != default_context_size {
                    agent.inner.context_size
                } else {
                    base.inner.context_size
                },
                top_p: agent.inner.top_p.or(base.inner.top_p),
                frequency_penalty: agent
                    .inner
                    .frequency_penalty
                    .or(base.inner.frequency_penalty),
                presence_penalty: agent.inner.presence_penalty.or(base.inner.presence_penalty),
                provider,
                parameters: if agent.inner.parameters.is_some() {
                    agent.inner.parameters.clone()
                } else {
                    base.inner.parameters.clone()
                },
                response_format: if agent.inner.response_format.is_some() {
                    agent.inner.response_format.clone()
                } else {
                    base.inner.response_format.clone()
                },
            },
        })
    }

    pub fn cleanup(&self) {
        // No-op: plugin registry has been removed
    }

    pub async fn get_configuration(&self) -> DistriServerConfig {
        self.configuration.read().await.clone()
    }

    pub fn configuration_handle(&self) -> Arc<RwLock<DistriServerConfig>> {
        self.configuration.clone()
    }

    pub async fn update_configuration(&self, configuration: DistriServerConfig) {
        let mut guard = self.configuration.write().await;
        *guard = configuration;
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
        // Try the store first
        if let Some(store) = &self.stores.prompt_template_store {
            // Find by name
            if let Ok(templates) = store.list().await {
                if let Some(record) = templates.into_iter().find(|t| t.name == name) {
                    return Some(crate::agent::prompt_registry::PromptTemplate {
                        name: record.name,
                        content: record.template,
                        description: record.description,
                        version: record.version,
                    });
                }
            }
        }

        // Fallback to registry
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

    pub async fn get_agent_tools(
        &self,
        definition: &crate::types::StandardDefinition,
        external_tools: &[Arc<dyn Tool>],
    ) -> Result<Vec<Arc<dyn Tool>>, AgentError> {
        // Use new tools configuration if available, fallback to old mcp_servers
        let tools_config = definition.tools.clone().unwrap_or(ToolsConfig::default());

        let mut tools = crate::tools::resolve_tools_config(
            &tools_config,
            self.mcp_registry.clone(),
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

        // Always include FinalTool if not already present
        let final_tool = Arc::new(FinalTool) as Arc<dyn Tool>;
        let has_final = tools.iter().any(|t| t.get_name() == final_tool.get_name());
        if !has_final {
            tools.push(final_tool);
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

        // Auto-include browser_agent sub-agent when browser_config.enabled = true
        // This allows any agent with browser enabled to delegate browser tasks to the specialized browser agent
        // Skip if this agent IS browser_agent (don't add itself as a sub-agent)
        let is_browser_agent =
            definition.name == "browser_agent" || definition.name.ends_with("/browser_agent");
        if definition.should_use_browser() && !is_browser_agent {
            let browser_agent_name = "browser_agent".to_string();
            // Check if browser_agent is not already in sub_agents
            let already_has_browser_agent = definition
                .sub_agents
                .iter()
                .any(|name| name == &browser_agent_name || name.ends_with("/browser_agent"));
            if !already_has_browser_agent {
                tools.push(Arc::new(crate::tools::AgentTool::new(browser_agent_name)));
                tracing::debug!(
                    "Auto-included browser_agent sub-agent for agent '{}' (browser_config.enabled = true)",
                    definition.name
                );
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

                // Register load_skill tool and available skills metadata if agent has skills configured
                if !definition.available_skills.is_empty() && self.stores.skill_store.is_some() {
                    // Resolve available skills - support wildcard "*"
                    let has_wildcard = definition
                        .available_skills
                        .iter()
                        .any(|s| s.id == "*" || s.name == "*");

                    let resolved_skills = if has_wildcard {
                        // Wildcard: load all skills from store
                        if let Some(skill_store) = self.stores.skill_store.as_ref() {
                            match skill_store.list_skills().await {
                                Ok(skills) => skills
                                    .into_iter()
                                    .map(|s| distri_types::AvailableSkill {
                                        id: s.id,
                                        name: s.name,
                                        description: s.description,
                                    })
                                    .collect(),
                                Err(e) => {
                                    tracing::warn!("Failed to load skills for wildcard: {}", e);
                                    vec![]
                                }
                            }
                        } else {
                            vec![]
                        }
                    } else {
                        definition.available_skills.clone()
                    };

                    // Filter out connection:* skills — they're surfaced via {{> connections}} partial
                    let resolved_skills: Vec<_> = resolved_skills
                        .into_iter()
                        .filter(|s| !s.name.starts_with("connection:"))
                        .collect();

                    if !resolved_skills.is_empty() {
                        // Build the available skills description for the prompt template
                        let skills_description = resolved_skills
                            .iter()
                            .map(|s| {
                                let desc = s.description.as_deref().unwrap_or("No description");
                                format!("- **{}** (id: `{}`): {}", s.name, s.id, desc)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        // Inject the skills list via dynamic_values so the {{> skills}} partial can render it
                        context
                            .merge_hook_prompt_state(crate::agent::context::HookPromptState {
                                dynamic_values: std::collections::HashMap::from([(
                                    "available_skills".to_string(),
                                    serde_json::Value::String(skills_description),
                                )]),
                                ..Default::default()
                            })
                            .await;

                        // Add skill tool: load_skill
                        context
                            .extend_tools(vec![Arc::new(crate::tools::skill_script::LoadSkillTool)
                                as Arc<dyn Tool>])
                            .await;
                    }
                }

                // Inject available sub-agents into prompt context if any are configured
                if !definition.sub_agents.is_empty() {
                    let mut sub_agent_lines = Vec::new();
                    for name in &definition.sub_agents {
                        let desc = if let Some(agent_cfg) = self.get_agent(name).await {
                            match agent_cfg {
                                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                                    def.description.clone()
                                }
                                distri_types::configuration::AgentConfig::WorkflowAgent(def) => {
                                    def.description.clone()
                                }
                            }
                        } else {
                            "Sub-agent".to_string()
                        };
                        let safe_name = name.replace('/', "__");
                        sub_agent_lines.push(format!(
                            "- **{}** (`call_{}` / `transfer_to_agent(\"{}\")`) — {}",
                            name, safe_name, name, desc
                        ));
                    }
                    context
                        .merge_hook_prompt_state(crate::agent::context::HookPromptState {
                            dynamic_values: std::collections::HashMap::from([(
                                "available_sub_agents".to_string(),
                                serde_json::Value::String(sub_agent_lines.join("\n")),
                            )]),
                            ..Default::default()
                        })
                        .await;
                }

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
            distri_types::configuration::AgentConfig::WorkflowAgent(definition) => {
                let agent = crate::agent::WorkflowAgent::new(definition);
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

        Self::apply_agent_overrides(
            &mut agent_config,
            definition_overrides,
            &context.default_model_settings,
        );
        Self::validate_agent_model(&agent_config)?;

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

        Self::apply_agent_overrides(
            &mut agent_config,
            definition_overrides,
            &context.default_model_settings,
        );
        Self::validate_agent_model(&agent_config)?;

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
            agent_id: None,
            parts_metadata: None,
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
        filter: &distri_types::stores::ThreadListFilter,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<distri_types::stores::ThreadListResponse, AgentError> {
        self.stores
            .thread_store
            .list_threads(filter, limit, offset)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))
    }

    pub async fn get_agents_by_usage(
        &self,
        search: Option<&str>,
    ) -> Result<Vec<distri_types::stores::AgentUsageInfo>, AgentError> {
        self.stores
            .thread_store
            .get_agents_by_usage(search)
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
                        user_id: None,
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
                    user_id: None,
                    external_id: None,
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

    /// Validate that the agent has a model provider configured after merging
    /// workspace defaults with agent-level settings. At least one must provide
    /// model settings.
    fn validate_agent_model(agent_config: &AgentConfig) -> Result<(), AgentError> {
        match agent_config {
            AgentConfig::StandardAgent(definition) => {
                if definition.model_settings.is_none() {
                    return Err(AgentError::InvalidConfiguration(
                        "No model configured. Please set a default model in Agent Settings → Default Model, or configure a model provider on the agent."
                            .to_string(),
                    ));
                }
            }
            AgentConfig::WorkflowAgent(_) => {
                // Workflow agents don't require model settings
            }
        }
        Ok(())
    }

    pub fn apply_agent_overrides(
        agent_config: &mut distri_types::configuration::AgentConfig,
        definition_overrides: Option<DefinitionOverrides>,
        default_model_settings: &Option<ModelSettings>,
    ) {
        let definition = match agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(ref mut def) => def,
            distri_types::configuration::AgentConfig::WorkflowAgent(_) => {
                // Workflow agents don't support overrides or model merging
                return;
            }
        };
        let merged = match (definition.model_settings.take(), default_model_settings) {
            (Some(agent_model), Some(base)) => {
                match Self::merge_model_settings(base, &agent_model) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        tracing::error!(
                            "Failed to merge model settings for agent '{}': {}",
                            definition.name,
                            e
                        );
                        Some(base.clone())
                    }
                }
            }
            (Some(agent_model), None) => Some(agent_model),
            (None, Some(base)) => Some(base.clone()),
            (None, None) => None,
        };
        definition.model_settings = merged;

        let default_analysis_settings = default_model_settings.clone();
        definition.analysis_model_settings = match (
            definition.analysis_model_settings.take(),
            &default_analysis_settings,
        ) {
            (Some(agent_analysis), Some(base)) => {
                Some(Self::merge_model_settings(base, &agent_analysis).unwrap_or(base.clone()))
            }
            (Some(agent_analysis), None) => Some(agent_analysis),
            (None, base) => base.clone(),
        };
        tracing::debug!("Applying definition overrides: {:?}", definition_overrides);
        if let Some(overrides) = definition_overrides {
            definition.apply_overrides(overrides);
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

        false
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
        use crate::types::ToolsConfig;
        use std::collections::HashMap;

        let builtin_tools = crate::tools::get_builtin_tools(
            self.workspace_filesystem.clone(),
            self.session_filesystem.clone(),
            true,
        );

        // Create a ToolsConfig that includes all available tools
        let all_tools_config = ToolsConfig {
            builtin: vec![], // Don't include builtin tools by default for /toolcall
            packages: HashMap::new(),
            mcp: Vec::new(),
            external: None,
        };

        // Use the standardized resolve_tools_config method
        let mut tools = crate::tools::resolve_tools_config(
            &all_tools_config,
            self.mcp_registry.clone(),
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
}

impl AgentOrchestrator {
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
        mut llm_def: LlmDefinition,
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

        // Load agent definition to get base model_settings if available
        // Only StandardAgent has model_settings; workflow agents don't
        if let Some(AgentConfig::StandardAgent(def)) = self.get_agent(&llm_def.name).await.as_ref()
        {
            // Merge: use agent's model_settings as base, override with request's model_settings
            if let (Some(base), Some(override_ms)) = (def.model_settings(), &llm_def.model_settings)
            {
                let final_model_settings = Self::merge_model_settings(base, override_ms)
                    .unwrap_or_else(|e| {
                        tracing::error!(
                            "Failed to merge model settings for LLM call '{}': {}",
                            llm_def.name,
                            e
                        );
                        override_ms.clone()
                    });
                llm_def.model_settings = Some(final_model_settings);
            } else if llm_def.model_settings.is_none() {
                llm_def.model_settings = def.model_settings().cloned();
            }
        }
        // If agent not found, use request's model_settings as-is

        if llm_def.model_settings.is_none() {
            anyhow::bail!(
                "No model configured. Please set a default model in Agent Settings → Default Model."
            );
        }

        let executor = crate::llm::create_llm_executor(
            llm_def,
            vec![],
            Arc::new(ctx),
            None,
            llm_context.label.clone(),
        )?;
        let response = executor.execute(&messages).await?;
        Ok(parse_structured_output(&response.content))
    }
}

fn parse_structured_output(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}
