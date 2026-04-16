use crate::{
    agent::{
        prompt_registry::PromptRegistry, todos::TodosTool, AgentEventType, BaseAgent, InvokeResult,
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
use distri_types::{browser::BrowsrClientConfig, configuration::StoreConfig, HookMutation};
use distri_types::{
    configuration::{DefinitionOverrides, ObjectStorageConfig},
    LLmContext, OrchestratorTrait,
};
use distri_types::{
    LlmDefinition, ModelSettings, Part, ServerMetadataWrapper, ToolCall, ToolsConfig,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub const SKILL_STORAGE_ROOT: &str = "storage/skills";

#[derive(Clone)]
pub struct AgentOrchestrator {
    pub mcp_registry: Arc<RwLock<McpServerRegistry>>,

    pub session_filesystem: Arc<FileSystem>,
    /// Optional workspace filesystem for HTTP file routes (not used by agent tools).
    /// Set by the hosting application if workspace file APIs are needed.
    pub workspace_filesystem: Option<Arc<FileSystem>>,
    pub browser_config: Arc<RwLock<BrowsrClientConfig>>,
    pub additional_tools: Arc<RwLock<HashMap<String, Vec<Arc<dyn Tool>>>>>,
    pub prompt_registry: Arc<PromptRegistry>,
    /// Store configuration for creating new session stores
    pub store_config: StoreConfig,
    /// All stores - use this instead of individual store fields
    pub stores: InitializedStores,
    pub hooks: Arc<RwLock<HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>>>,
    pub inline_hooks: Arc<dashmap::DashMap<String, tokio::sync::oneshot::Sender<HookMutation>>>,
    pub hook_registry: HookRegistry,
    pub system_hooks: Vec<Arc<dyn crate::agent::types::AgentHooks>>,

    /// Optional background runner for async agent execution (deepagent containers).
    pub background_runner: Option<Arc<dyn crate::runner::BackgroundRunner>>,
    /// Unified runtime for event broadcasting + task coordination.
    /// Always initialized — InProcessRuntime by default, RedisRuntime for cloud.
    pub runtime: Arc<dyn crate::broadcast::AgentRuntime>,
}

impl std::fmt::Debug for AgentOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOrchestrator").finish()
    }
}

impl AgentOrchestrator {
    /// Shorthand for `self.runtime.broadcaster()`.
    pub fn broadcaster(&self) -> &dyn crate::broadcast::AgentEventBroadcaster {
        self.runtime.broadcaster()
    }

    /// Shorthand for `self.runtime.coordinator()`.
    pub fn coordinator(&self) -> &dyn crate::broadcast::AgentTaskCoordinator {
        self.runtime.coordinator()
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
    session_filesystem: Option<Arc<FileSystem>>,
    session_storage_path: Option<std::path::PathBuf>,
    workspace_filesystem: Option<Arc<FileSystem>>,
    browser_config: Option<BrowsrClientConfig>,
    stores: Option<InitializedStores>,
    prompt_registry: Option<Arc<PromptRegistry>>,
    store_config: Option<StoreConfig>,
    hooks: Option<HashMap<String, Arc<dyn crate::agent::types::AgentHooks>>>,
    system_hooks: Vec<Arc<dyn crate::agent::types::AgentHooks>>,
    runtime: Option<Arc<dyn crate::broadcast::AgentRuntime>>,
    background_runner: Option<Arc<dyn crate::runner::BackgroundRunner>>,
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

    pub fn with_session_storage_path(mut self, path: std::path::PathBuf) -> Self {
        self.session_storage_path = Some(path);
        self
    }

    pub fn with_session_filesystem(mut self, fs: Arc<FileSystem>) -> Self {
        self.session_filesystem = Some(fs);
        self
    }

    /// Set an optional workspace filesystem for HTTP file routes.
    /// This is NOT used by agent tools — only by workspace file API endpoints.
    pub fn with_workspace_filesystem(mut self, fs: Arc<FileSystem>) -> Self {
        self.workspace_filesystem = Some(fs);
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

    pub fn with_system_hooks(
        mut self,
        hooks: Vec<Arc<dyn crate::agent::types::AgentHooks>>,
    ) -> Self {
        self.system_hooks = hooks;
        self
    }
    pub fn with_runtime(mut self, runtime: Arc<dyn crate::broadcast::AgentRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn with_background_runner(
        mut self,
        runner: Arc<dyn crate::runner::BackgroundRunner>,
    ) -> Self {
        self.background_runner = Some(runner);
        self
    }

    pub async fn build(self) -> anyhow::Result<AgentOrchestrator> {
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

        // Create session filesystem for internal artifact/large-response processing.
        // This is independent of any workspace — it's purely for session-scoped storage.
        let session_filesystem = if let Some(fs) = self.session_filesystem {
            fs
        } else {
            let session_path = self
                .session_storage_path
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp/distri-sessions"));

            let fs_config = distri_filesystem::FileSystemConfig {
                object_store: ObjectStorageConfig::FileSystem {
                    base_path: session_path.to_string_lossy().to_string(),
                },
                root_prefix: None,
            };

            Arc::new(distri_filesystem::create_file_system(fs_config).await?)
        };

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

        // Auto-create InProcessRuntime from TaskStore if no runtime was provided
        let runtime = self.runtime.unwrap_or_else(|| {
            Arc::new(crate::broadcast::in_process::InProcessRuntime::new(
                stores.task_store.clone(),
            ))
        });

        let orchestrator = AgentOrchestrator {
            mcp_registry: registry,
            session_filesystem,
            workspace_filesystem: self.workspace_filesystem,
            browser_config,
            additional_tools: Arc::new(RwLock::new(self.additional_tools.unwrap_or_default())),
            prompt_registry,
            store_config,
            stores,
            system_hooks: self.system_hooks,
            hooks: hooks.clone(),
            inline_hooks: Arc::new(dashmap::DashMap::new()),
            hook_registry: HookRegistry::new(),
            runtime,
            background_runner: self.background_runner,
        };

        // Sync system prompts to the store
        if let Some(store) = &orchestrator.stores.prompt_template_store {
            let defaults = PromptRegistry::get_default_templates();
            if let Err(e) = store.sync_system_templates(defaults).await {
                tracing::warn!("⚠️  Failed to sync system prompts to store: {}", e);
            }
        }

        // Default system agents (distri, distri_runner, distri_browser_runner)
        // are seeded by cloud/src/state.rs::seed_default_agents() on startup,
        // not by the orchestrator. This keeps the orchestrator generic —
        // callers decide which agents live in their store.

        Ok(orchestrator)
    }
}

impl AgentOrchestrator {

    pub fn cleanup(&self) {
        // No-op: plugin registry has been removed
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
    ) -> Result<crate::tools::ResolvedTools, AgentError> {
        // Use new tools configuration if available, fallback to old mcp_servers
        let tools_config = definition.tools.clone().unwrap_or(ToolsConfig::default());

        let mut resolved = crate::tools::resolve_tools_with_deferral(
            &tools_config,
            self.mcp_registry.clone(),
            external_tools,
        )
        .await
        .map_err(|e| AgentError::ToolExecution(e.to_string()))?;

        let mut tools = std::mem::take(&mut resolved.all_tools);

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

        // Always register UniversalAgentTool — every agent can delegate
        let has_call_agent = tools.iter().any(|t| t.get_name() == "call_agent");
        if !has_call_agent {
            tools.push(Arc::new(crate::tools::UniversalAgentTool));
        }

        // Always keep transfer_to_agent (access control checked at execution time)

        let is_browser_agent =
            definition.name == "browser_agent" || definition.name.ends_with("/browser_agent");
        if definition.should_use_browser() && !is_browser_agent {
            let browser_agent_name = "browser_agent".to_string();
            let already_has_browser_agent = definition
                .sub_agents
                .iter()
                .any(|name| name == &browser_agent_name || name.ends_with("/browser_agent"));
            if !already_has_browser_agent {
                tracing::debug!(
                    "browser_agent available for '{}' via call_agent (browser_config.enabled = true)",
                    definition.name
                );
            }
        }

        resolved.all_tools = tools;
        Ok(resolved)
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
                let resolved = self.get_agent_tools(&definition, &external_tools).await?;
                let deferred_names: std::collections::HashSet<String> = resolved
                    .deferred_tools
                    .iter()
                    .map(|t| t.name.clone())
                    .collect();
                context.set_deferred_tool_names(deferred_names).await;
                if let Some(listing) = resolved.deferred_tools_listing() {
                    context
                        .merge_hook_prompt_state(crate::agent::context::HookPromptState {
                            dynamic_values: std::collections::HashMap::from([(
                                "deferred_tools_listing".to_string(),
                                serde_json::Value::String(listing),
                            )]),
                            ..Default::default()
                        })
                        .await;
                }
                context.extend_tools(resolved.all_tools).await;

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
                            match skill_store
                                .list(distri_types::stores::SkillFilter {
                                    scope: distri_types::stores::SkillScope::All,
                                    ..Default::default()
                                })
                                .await
                            {
                                Ok(resp) => resp
                                    .skills
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
                        // Build the available skills description using budget-capped listing
                        let frontmatters: Vec<distri_types::stores::SkillFrontmatter> =
                            resolved_skills
                                .iter()
                                .map(|s| distri_types::stores::SkillFrontmatter {
                                    name: s.name.clone(),
                                    description: s.description.clone(),
                                    ..Default::default()
                                })
                                .collect();

                        let skills_description = distri_types::stores::format_skill_listing(
                            &frontmatters,
                            distri_types::stores::SKILL_LISTING_BUDGET,
                        );

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

                // Inject connection context for connection-aware agents
                {
                    let is_connection_aware = definition.available_skills.iter().any(|s| {
                        s.name.contains("connections_manager")
                            || s.name.starts_with("connection:")
                            || s.name == "*"
                    });

                    if is_connection_aware {
                        if let Some(ws_id) = &context.workspace_id {
                            let mut dv = std::collections::HashMap::new();

                            // Build connected services list
                            if let Some(conn_store) = &self.stores.connection_store {
                                if let Ok(connections) = conn_store.list_by_workspace(ws_id).await {
                                    let connected_names: std::collections::HashSet<String> =
                                        connections.iter().map(|c| c.name.clone()).collect();

                                    let connected_lines: Vec<String> = connections
                                        .iter()
                                        .filter(|c| {
                                            c.status
                                                == distri_types::connections::ConnectionStatus::Connected
                                        })
                                        .map(|c| {
                                            format!(
                                                "- **{}** (id: `{}`, status: connected)",
                                                c.name, c.id
                                            )
                                        })
                                        .collect();

                                    if !connected_lines.is_empty() {
                                        dv.insert(
                                            "available_connections".to_string(),
                                            serde_json::Value::String(connected_lines.join("\n")),
                                        );
                                    }

                                    // Build available-but-not-connected providers list
                                    if let Some(registry) = &self.stores.provider_registry {
                                        let all_providers = registry.list_providers().await;
                                        let mut available_lines = Vec::new();
                                        for provider in &all_providers {
                                            if !connected_names.contains(provider)
                                                && registry.is_provider_available(provider).await
                                            {
                                                available_lines.push(format!(
                                                    "- **{}** — ready to connect via OAuth",
                                                    provider
                                                ));
                                            }
                                        }
                                        if !available_lines.is_empty() {
                                            dv.insert(
                                                "available_providers".to_string(),
                                                serde_json::Value::String(
                                                    available_lines.join("\n"),
                                                ),
                                            );
                                        }
                                    }
                                }
                            }

                            if !dv.is_empty() {
                                context
                                    .merge_hook_prompt_state(
                                        crate::agent::context::HookPromptState {
                                            dynamic_values: dv,
                                            ..Default::default()
                                        },
                                    )
                                    .await;
                            }
                        }
                    }
                }

                // Always inject agent delegation info into prompt context
                {
                    let mut sub_agent_lines = Vec::new();

                    // Always-available system agents
                    for builtin_name in crate::tools::builtin::ALWAYS_AVAILABLE_BUILTINS {
                        if let Some(agent_cfg) = self.get_agent(builtin_name).await {
                            let desc = match agent_cfg {
                                distri_types::configuration::AgentConfig::StandardAgent(def) => {
                                    def.description.clone()
                                }
                                distri_types::configuration::AgentConfig::WorkflowAgent(def) => {
                                    def.description.clone()
                                }
                            };
                            let short_name = builtin_name
                                .strip_prefix("_system/")
                                .unwrap_or(builtin_name);
                            sub_agent_lines.push(format!(
                                "- **{}** — {} *(always available)*",
                                short_name, desc
                            ));
                        }
                    }

                    // Declared sub_agents (store agents + opt-in built-ins)
                    for name in &definition.sub_agents {
                        if name == "*" {
                            sub_agent_lines.push(
                                "- **\\*** — all agents in the workspace are available".to_string(),
                            );
                            continue;
                        }
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
                        sub_agent_lines.push(format!("- **{}** — {}", name, desc));
                    }

                    // Always mention transfer_to_agent availability
                    sub_agent_lines.push(
                        "\n*Use `transfer_to_agent` to hand over control completely (your execution stops, target agent takes over with your history).*".to_string()
                    );

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
                    let mut hooks: Vec<Arc<dyn crate::agent::types::AgentHooks>> =
                        self.system_hooks.clone(); // system hooks always run first

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
                let hooks: Arc<dyn crate::agent::types::AgentHooks> = Arc::new(
                    crate::agent::hooks::CombinedHooks::new(self.system_hooks.clone()),
                );
                let agent = crate::agent::WorkflowAgent::new(definition, hooks);
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

    pub(crate) async fn call_agent_stream(
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

        // Wildcard external-tools sanity check: an agent that declares
        // `external = ["*"]` is asking the client to ship at least one tool.
        // An empty client list silently produces an LLM with zero external tools
        // and confusing downstream errors — fail fast at request entry.
        if let distri_types::configuration::AgentConfig::StandardAgent(ref def) = agent_config {
            if let Some(tools_cfg) = def.tools.as_ref() {
                if let Some(ext) = tools_cfg.external.as_ref() {
                    if ext.iter().any(|t| t == "*") && context.external_tools_count().await == 0 {
                        return Err(AgentError::Session(format!(
                            "Agent '{}' declares external = [\"*\"] but no external tools \
                             were provided by the client. The client must ship at least one \
                             external tool in ExecutorContextMetadata.external_tools.",
                            def.name
                        )));
                    }
                }
            }
        }

        // Runtime-constraint dispatch. If the agent declares any runtime
        // constraints and the current ExecutorContext.runtime_mode is not in
        // the allowed list, route through RemoteAgent — but only if a
        // BackgroundRunner is configured whose provided_runtime is in the
        // allowed list. Otherwise fail fast with a clear error.
        //
        // Note: this check runs BEFORE `validate_agent_model` because
        // remote-dispatched agents configure their model inside the sandbox
        // (via the inner distri-cli's own settings) — the outer orchestrator
        // does not need a model for the dispatch path.
        if let distri_types::configuration::AgentConfig::StandardAgent(ref definition) =
            agent_config
        {
            let allowed = definition.allowed_runtimes();
            if !allowed.is_empty() && !allowed.iter().any(|rt| rt == &context.runtime_mode) {
                let Some(runner) = &self.background_runner else {
                    return Err(AgentError::Session(format!(
                        "Agent '{}' requires runtime {:?} but the current runtime is {:?} \
                         and no background runner is configured to provide it.",
                        definition.name, allowed, context.runtime_mode
                    )));
                };
                let provided = runner.provided_runtime();
                if !allowed.iter().any(|rt| rt == &provided) {
                    return Err(AgentError::Session(format!(
                        "Agent '{}' requires runtime {:?} but the only available background \
                         runner provides {:?}.",
                        definition.name, allowed, provided
                    )));
                }
                let hooks: Arc<dyn crate::agent::types::AgentHooks> = Arc::new(
                    crate::agent::hooks::CombinedHooks::new(self.system_hooks.clone()),
                );
                let agent = crate::agent::remote::RemoteAgent {
                    definition: definition.clone(),
                    runner: runner.clone(),
                    broadcaster: self.runtime.broadcaster_arc(),
                    hooks,
                };
                return agent.invoke_stream(message, context).await;
            }
        }

        // In-process path requires a configured model. Remote agents skipped
        // this above because their model lives inside the sandbox.
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
        // Same wildcard validation as call_agent_stream — inline agents must
        // also receive at least one external tool when declaring `external = ["*"]`.
        if let distri_types::configuration::AgentConfig::StandardAgent(ref def) = agent_config {
            if let Some(tools_cfg) = def.tools.as_ref() {
                if let Some(ext) = tools_cfg.external.as_ref() {
                    if ext.iter().any(|t| t == "*") && context.external_tools_count().await == 0 {
                        return Err(AgentError::Session(format!(
                            "Agent '{}' declares external = [\"*\"] but no external tools \
                             were provided by the client. The client must ship at least one \
                             external tool in ExecutorContextMetadata.external_tools.",
                            def.name
                        )));
                    }
                }
            }
        }

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
        channel_id: Option<String>,
        thread_store: &Arc<dyn ThreadStore>,
    ) -> Result<Thread, AgentError> {
        // For ephemeral mode, skip the get_thread check and always create new thread
        // This avoids "failed to load thread" errors with fresh ephemeral databases
        let thread = if self.is_ephemeral() {
            tracing::info!(is_ephemeral=true, ?thread_id, "ensure_thread: skipping lookup (ephemeral)");
            None
        } else {
            match &thread_id {
                Some(tid) => {
                    let found = thread_store
                        .get_thread(tid)
                        .await
                        .map_err(|e| AgentError::Session(e.to_string()))?;
                    tracing::info!(
                        requested_thread_id = %tid,
                        found = found.is_some(),
                        "ensure_thread: get_thread result"
                    );
                    found
                }
                None => {
                    tracing::info!("ensure_thread: no thread_id provided, creating new");
                    None
                }
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
                    channel_id,
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
        channel_id: Option<String>,
    ) -> Result<Thread, AgentError> {
        self.ensure_thread_exists_with_store(
            agent_id,
            thread_id,
            title,
            attributes,
            channel_id,
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
            context.channel_id.clone(),
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

        // Look up parent run for OTel span nesting (set by RemoteAgent before spawning inner task).
        let context = if context.parent_run_id.is_none() {
            if let Ok(Some(parent_run_id)) =
                self.broadcaster().get_parent_run(&context.task_id).await
            {
                let mut ctx = (*context).clone();
                ctx.parent_run_id = Some(parent_run_id);
                Arc::new(ctx)
            } else {
                context
            }
        } else {
            context
        };

        self.validate_user_message(&message)?;

        self.call_agent(agent_name, message, context, definition_overrides)
            .await
    }

    // ── Background execution helpers ──────────────────────────────────

    /// Register a task with the coordinator, wire cancellation signal and mailbox
    /// into the execution context. Returns the wired context and task metadata.
    ///
    /// This is the first step in the background execution flow:
    /// 1. `register_task()` — register + wire context (this method)
    /// 2. `spawn_task_relay()` — spawn event relay to broadcaster
    /// 3. `subscribe_to_task()` — subscribe to broadcaster events as SSE
    pub async fn register_task(
        &self,
        task_id: &str,
        thread_id: &str,
        exec_ctx: ExecutorContext,
    ) -> anyhow::Result<(
        Arc<ExecutorContext>,
        tokio::sync::mpsc::Receiver<distri_types::AgentEvent>,
    )> {
        let coordinator = self.coordinator();

        let cancel_signal = coordinator.register_task(task_id, thread_id, None).await?;

        let mailbox = coordinator.take_mailbox(task_id).await.ok();

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<distri_types::AgentEvent>(100);
        let mut wired_ctx = exec_ctx.clone_with_tx(event_tx);
        wired_ctx.cancellation_signal = Some(cancel_signal);
        if let Some(mb) = mailbox {
            wired_ctx.mailbox = Some(Arc::new(tokio::sync::Mutex::new(mb)));
        }

        Ok((Arc::new(wired_ctx), event_rx))
    }

    /// Spawn the event relay task: receives events from the agent's event channel
    /// and publishes them to the broadcaster. Completes the task in the coordinator
    /// when a terminal event is received. Auto-completes inline hooks.
    pub fn spawn_task_relay(
        &self,
        task_id: String,
        mut event_rx: tokio::sync::mpsc::Receiver<distri_types::AgentEvent>,
    ) {
        let runtime = self.runtime.clone();
        let executor = Arc::new(self.clone());

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let is_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                );

                // Auto-complete inline hooks
                if let AgentEventType::InlineHookRequested { ref request } = event.event {
                    let _ = executor
                        .complete_inline_hook(&request.hook_id, HookMutation::none())
                        .await;
                }

                // Publish to broadcaster
                let event_task_id = event.task_id.clone();
                let _ = runtime.broadcaster().publish(&event_task_id, event).await;

                if is_terminal {
                    let _ = runtime.coordinator().complete_task(&task_id).await;
                    break;
                }
            }
        });
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
            context.channel_id.clone(),
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

        // Look up parent run for OTel span nesting (set by RemoteAgent before spawning inner task).
        let context = if context.parent_run_id.is_none() {
            if let Ok(Some(parent_run_id)) =
                self.broadcaster().get_parent_run(&context.task_id).await
            {
                let mut ctx = (*context).clone();
                ctx.parent_run_id = Some(parent_run_id);
                Arc::new(ctx)
            } else {
                context
            }
        } else {
            context
        };

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
                match base.merge(&agent_model) {
                    Some(m) => Some(m),
                    None => {
                        tracing::error!(
                            "merge produced empty model for agent '{}'",
                            definition.name,
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
                base.merge(&agent_analysis).or_else(|| Some(base.clone()))
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

    /// Resolve an agent identifier (UUID or name) to the canonical agent name.
    /// Returns the original identifier if the agent is not found.
    pub async fn resolve_agent_name(&self, id_or_name: &str) -> String {
        if let Some(agent) = self.get_agent(id_or_name).await {
            agent.get_name().to_string()
        } else {
            id_or_name.to_string()
        }
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

        let builtin_tools = crate::tools::get_builtin_tools();

        // Create a ToolsConfig that includes all available tools
        let all_tools_config = ToolsConfig {
            builtin: vec![], // Don't include builtin tools by default for /toolcall
            ..Default::default()
        };

        // Use the standardized resolve_tools_config method
        let mut tools =
            crate::tools::resolve_tools_config(&all_tools_config, self.mcp_registry.clone(), &[])
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
                let final_model_settings = base.merge(override_ms).unwrap_or_else(|| {
                    tracing::error!(
                        "merge produced empty model for LLM call '{}'",
                        llm_def.name
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
