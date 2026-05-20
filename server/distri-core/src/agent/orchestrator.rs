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
use distri_auth::OAuthHandler;
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
    pub remote_task_runner: Option<Arc<dyn crate::runner::RemoteTaskRunner>>,
    /// Unified runtime for event broadcasting + task coordination.
    /// Always initialized — InProcessRuntime by default, RedisRuntime for cloud.
    pub runtime: Arc<dyn crate::broadcast::AgentRuntime>,
    /// Optional OAuth handler for the connections OAuth flow.
    /// Present only when OAuth provider credentials have been configured.
    pub oauth_handler: Option<Arc<OAuthHandler>>,
    /// Optional MCP pool provider. When set, `create_agent_from_config` asks
    /// the provider for a per-run `McpClientPool` and threads it into tool
    /// resolution. Cloud sets this (Postgres-backed connection store + the
    /// `DefaultResolver` auth pipeline); the standalone OSS server leaves it
    /// `None` and uses the static `[[tools.mcp]]` registry only.
    pub mcp_pool_provider: Option<Arc<dyn crate::servers::McpPoolProvider>>,
    /// Workflow execution-state store — one trait covering both
    /// run-level state (definition snapshot, entry point, input,
    /// shared context) and per-step state (status, result, error,
    /// optional `wait_task_id` for A2A-addressable wait steps).
    /// `None` when not wired (OSS tests etc.); the cloud sets an
    /// `InMemoryWorkflowStore`/`RedisWorkflowStore` here.
    pub workflow_store: Option<Arc<dyn distri_workflow::WorkflowStore>>,
    /// Routing index from declared `WorkflowTrigger`s back to
    /// `(agent_id, entry_point_id)`. Built on boot from every
    /// `WorkflowAgentDefinition`; what the cloud's webhook route,
    /// scheduler tick, event bus, and workflow-as-tool A2A dispatch
    /// consult to find the workflow run a stimulus targets.
    pub workflow_trigger_registry:
        Option<Arc<dyn distri_workflow::WorkflowTriggerRegistry>>,
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
    remote_task_runner: Option<Arc<dyn crate::runner::RemoteTaskRunner>>,
    oauth_handler: Option<Arc<OAuthHandler>>,
    mcp_pool_provider: Option<Arc<dyn crate::servers::McpPoolProvider>>,
    workflow_store: Option<Arc<dyn distri_workflow::WorkflowStore>>,
    workflow_trigger_registry:
        Option<Arc<dyn distri_workflow::WorkflowTriggerRegistry>>,
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

    pub fn with_remote_task_runner(
        mut self,
        runner: Arc<dyn crate::runner::RemoteTaskRunner>,
    ) -> Self {
        self.remote_task_runner = Some(runner);
        self
    }

    /// Attach an OAuth handler for the connections OAuth flow.
    /// When set, `POST /connections` with OAuth auth_type will generate a real
    /// authorization URL and `POST /connections/oauth/callback` will exchange codes.
    pub fn with_oauth_handler(mut self, handler: Arc<OAuthHandler>) -> Self {
        self.oauth_handler = Some(handler);
        self
    }

    /// Attach an `McpPoolProvider`. When set, every agent run's tool
    /// resolution asks the provider for a per-run `McpClientPool` keyed by
    /// the context's workspace + user, so every entry-point (cloud gateway,
    /// JSON-RPC CLI, tests) sees the same MCP tools.
    pub fn with_mcp_pool_provider(
        mut self,
        provider: Arc<dyn crate::servers::McpPoolProvider>,
    ) -> Self {
        self.mcp_pool_provider = Some(provider);
        self
    }

    /// Attach the workflow execution-state store used by
    /// `WorkflowAgent`. One store, covering both run-level and
    /// step-level state; cloud wires `RedisWorkflowStore`, OSS/tests
    /// use `InMemoryWorkflowStore`.
    pub fn with_workflow_store(
        mut self,
        store: Arc<dyn distri_workflow::WorkflowStore>,
    ) -> Self {
        self.workflow_store = Some(store);
        self
    }

    /// Attach the workflow trigger registry — the routing index from
    /// declared triggers (webhook path / cron / event topic / tool
    /// name) back to `(agent_id, entry_point_id)`. The cloud builds
    /// it on boot from every `WorkflowAgentDefinition`.
    pub fn with_workflow_trigger_registry(
        mut self,
        registry: Arc<dyn distri_workflow::WorkflowTriggerRegistry>,
    ) -> Self {
        self.workflow_trigger_registry = Some(registry);
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
            remote_task_runner: self.remote_task_runner,
            oauth_handler: self.oauth_handler,
            mcp_pool_provider: self.mcp_pool_provider,
            workflow_store: self.workflow_store,
            workflow_trigger_registry: self.workflow_trigger_registry,
        };

        // Sync system prompts to the store
        if let Some(store) = &orchestrator.stores.prompt_template_store {
            let defaults = PromptRegistry::get_default_templates();
            if let Err(e) = store.sync_system_templates(defaults).await {
                tracing::warn!("⚠️  Failed to sync system prompts to store: {}", e);
            }
        }

        // Default bundled agents: cloud uses `seed_default_agents()`; OSS
        // `distri-server-cli` calls `seed::seed_bundled_defaults()` after build.
        // The orchestrator stays generic — callers decide store population.

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

    /// Resolve the per-run MCP pool for an `ExecutorContext` via the attached
    /// provider. Called from inside `create_agent_from_config`, where tool
    /// resolution happens — this is the single place a run's MCP pool comes
    /// from. Returns `None` when no provider is configured (OSS standalone)
    /// or when the provider declines.
    pub async fn resolve_mcp_pool(
        &self,
        ctx: &ExecutorContext,
    ) -> Option<Arc<crate::servers::McpClientPool>> {
        let provider = self.mcp_pool_provider.as_ref()?;
        provider.build_pool(ctx).await
    }

    pub async fn register_agent_definition(
        &self,
        definition: crate::types::StandardDefinition,
    ) -> anyhow::Result<()> {
        let config = distri_types::configuration::AgentConfig::StandardAgent(definition);
        self.register_agent_config(config).await
    }

    /// Variant-aware registration. Workflow agents skip the partial-template
    /// hookup (workflows have no prompt templates), but still go through the
    /// store so they appear in `list_agents` and the workspace tree.
    pub async fn register_agent_config(
        &self,
        config: distri_types::configuration::AgentConfig,
    ) -> anyhow::Result<()> {
        tracing::debug!("🤖 Registering agent config: {}", config.get_name());
        if let distri_types::configuration::AgentConfig::StandardAgent(ref definition) = config {
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
                    }
                }
            }
        }

        self.stores
            .agent_store
            .register(config)
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
        self.get_agent_tools_with_pool(definition, external_tools, None)
            .await
    }

    /// Pool-aware variant: include MCP tools discovered through the pool.
    pub async fn get_agent_tools_with_pool(
        &self,
        definition: &crate::types::StandardDefinition,
        external_tools: &[Arc<dyn Tool>],
        mcp_pool: Option<Arc<crate::servers::McpClientPool>>,
    ) -> Result<crate::tools::ResolvedTools, AgentError> {
        // Use new tools configuration if available, fallback to old mcp_servers
        let tools_config = definition.tools.clone().unwrap_or(ToolsConfig::default());

        let mut resolved = crate::tools::resolve_tools_with_deferral_and_pool(
            &tools_config,
            self.mcp_registry.clone(),
            external_tools,
            mcp_pool,
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

        // If any tools were deferred, the model needs `tool_search` to
        // discover and load their schemas. Auto-inject it — agents that
        // don't list it explicitly still get the load-on-demand path.
        if !resolved.deferred_tools.is_empty() {
            let search_tool = Arc::new(crate::tools::ToolSearchTool) as Arc<dyn Tool>;
            let has_search = tools.iter().any(|t| t.get_name() == search_tool.get_name());
            if !has_search {
                tools.push(search_tool);
            }
        }

        // Auto-register InvokeAgentTool ONLY when the agent definition's
        // `tools.builtin` is empty or unset. An agent that explicitly
        // enumerates its builtins (e.g. `_adhoc_base` with
        // `builtin = ["final"]`, or a worker dispatched via
        // `invoke_agent({tools: { kind: "exact", tools: ["final"] }})`) is
        // opting out of delegation by design — adding `invoke_agent` back
        // would re-enable the recursion loop the explicit list was meant
        // to block.
        let has_invoke = tools.iter().any(|t| t.get_name() == "invoke_agent");
        let has_explicit_builtin = definition
            .tools
            .as_ref()
            .map(|t| !t.builtin.is_empty())
            .unwrap_or(false);
        if !has_invoke && !has_explicit_builtin {
            tools.push(Arc::new(crate::tools::InvokeAgentTool));
        }

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

                // Resolve the per-run MCP pool here, inside the tool-building
                // path that every agent run flows through. The orchestrator's
                // attached `McpPoolProvider` (set by the host application —
                // cloud wires it from PgConnectionStore + DefaultResolver) is
                // the *single* source of truth for which MCP servers a run
                // sees. Standalone hosts that don't attach a provider get
                // `None` and the static `[[tools.mcp]]` registry still works.
                let mcp_pool = self.resolve_mcp_pool(&context).await;
                let resolved = self
                    .get_agent_tools_with_pool(&definition, &external_tools, mcp_pool)
                    .await?;
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

                        // Add `load_skill` for inline skill reference
                        // (loads body into current agent context). Per the
                        // claude-code split, "use skill X in a sub-agent"
                        // is the parent calling `invoke_agent` with a
                        // prompt that says "load skill X and do Y" —
                        // sub-agent then calls `load_skill` itself.
                        context
                            .extend_tools(vec![Arc::new(
                                crate::tools::skill_script::LoadSkillTool,
                            )
                                as Arc<dyn Tool>])
                            .await;
                    }
                }

                // Resolve declared connections. Agents must declare connections
                // in `definition.connections` to get any injection — we do NOT
                // auto-discover or auto-inject based on skills or workspace
                // state. This keeps the prompt/env surface explicit.
                if !definition.connections.is_empty() {
                    resolve_declared_connections(&self.stores, &definition, &context).await?;
                }

                // Always inject agent delegation info into prompt context
                {
                    let mut sub_agent_lines = Vec::new();

                    // Always-available system agents
                    for builtin_name in crate::tools::invoke_agent::ALWAYS_AVAILABLE_BUILTINS {
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

                    // Track names we've already listed so explicit entries and
                    // the wildcard expansion don't collide.
                    let mut listed: std::collections::HashSet<String> =
                        crate::tools::invoke_agent::ALWAYS_AVAILABLE_BUILTINS
                            .iter()
                            .map(|s| s.to_string())
                            .collect();

                    // Declared sub_agents (store agents + opt-in built-ins).
                    // `"*"` expands to every non-system agent in the workspace
                    // (minus self and the always-available builtins already
                    // listed above). This mirrors `available_skills = [{id:"*"}]`.
                    for name in &definition.sub_agents {
                        if name == "*" {
                            let (all_agents, _) = self.list_agents(None, None).await;
                            for cfg in &all_agents {
                                let (agent_name, desc) = match cfg {
                                    distri_types::configuration::AgentConfig::StandardAgent(
                                        def,
                                    ) => (def.name.clone(), def.description.clone()),
                                    distri_types::configuration::AgentConfig::WorkflowAgent(
                                        def,
                                    ) => (def.name.clone(), def.description.clone()),
                                };
                                if agent_name == definition.name || listed.contains(&agent_name) {
                                    continue;
                                }
                                sub_agent_lines.push(format!("- **{}** — {}", agent_name, desc));
                                listed.insert(agent_name);
                            }
                            continue;
                        }
                        if listed.contains(name) {
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
                        listed.insert(name.clone());
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

                // Populate the trigger registry from this workflow's
                // entry-point triggers so webhook/cron/event/tool
                // dispatch can route back to it. Best-effort; logs
                // and continues on failure (the registry is the
                // orchestration index, not the source of truth).
                if let Some(registry) = self.workflow_trigger_registry.clone() {
                    if let Ok(workflow_def) = serde_json::from_value::<
                        distri_workflow::WorkflowDefinition,
                    >(definition.definition.clone())
                    {
                        if let Err(e) =
                            registry.register(&definition.name, &workflow_def).await
                        {
                            tracing::warn!(
                                error = %e,
                                agent_id = %definition.name,
                                "workflow_trigger_registry register failed"
                            );
                        }
                    }
                }

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
        self.hydrate_agent_model_settings(&mut agent_config).await?;
        Self::validate_agent_model(&agent_config)?;

        let declared_definition = match &agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(def) => Some(def.clone()),
            _ => None,
        };

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

        if let Some(def) = declared_definition {
            warn_unused_connections(&def, &context).await;
        }

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
        self.hydrate_agent_model_settings(&mut agent_config).await?;

        // Runtime-constraint dispatch decision. Single source of truth
        // lives in `crate::agent::invoke::decide_dispatch` — both this
        // legacy entry and the typed `invoke()` entry route through it,
        // so the local-vs-remote logic stays consistent.
        //
        // **Order matters.** This must run BEFORE the wildcard
        // external-tools check below: remote-dispatched agents have
        // their tools satisfied by the inner distri-cli inside the
        // sandbox, so the outer orchestrator must NOT validate the
        // wildcard locally — it would wrongly reject `_adhoc_base`-style
        // agents (declared with `external = ["*"]` and `runtime =
        // ["cli"]`) just because the cloud client doesn't ship any
        // tools. Same reasoning as the `validate_agent_model` skip:
        // the model and the tools both live inside the sandbox.
        if let distri_types::configuration::AgentConfig::StandardAgent(ref definition) =
            agent_config
        {
            let plan = crate::agent::invoke::decide_dispatch(
                definition,
                &context.runtime_mode,
                &distri_types::invocation::ExecutorHint::Auto,
                self.remote_task_runner.as_ref(),
            )?;
            if let crate::agent::invoke::DispatchPlan::Remote { runner } = plan {
                let hooks: Arc<dyn crate::agent::types::AgentHooks> = Arc::new(
                    crate::agent::hooks::CombinedHooks::new(self.system_hooks.clone()),
                );
                let agent = crate::agent::remote::RemoteAgent {
                    definition: definition.clone(),
                    runner,
                    broadcaster: self.runtime.broadcaster_arc(),
                    hooks,
                };
                return agent.invoke_stream(message, context).await;
            }
        }

        // Wildcard external-tools sanity check (in-process path only): an
        // agent that declares `external = ["*"]` is asking the client to
        // ship at least one tool. An empty client list silently produces an
        // LLM with zero external tools and confusing downstream errors —
        // fail fast at request entry.
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

    // The unified `invoke()` entry point for typed Invocation dispatch
    // lives in the sibling `agent::invoke` module — see that file for
    // the impl block. Method resolution finds `orch.invoke(...)` there.

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
        let thread = self
            .stores
            .thread_store
            .get_thread(thread_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Compute `active_task_id` at read time: the first non-terminal task
        // in this thread. Never persisted to the DB; clients use it to decide
        // whether to resubscribe on thread reopen.
        if let Some(mut t) = thread {
            if let Ok(tasks) = self.stores.task_store.list_tasks(Some(thread_id)).await {
                t.active_task_id = tasks
                    .into_iter()
                    .find(|task| !task.status.is_terminal())
                    .map(|task| task.id);
            }
            Ok(Some(t))
        } else {
            Ok(None)
        }
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
            tracing::info!(
                is_ephemeral = true,
                ?thread_id,
                "ensure_thread: skipping lookup (ephemeral)"
            );
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
        let thread_store = self.stores.thread_store.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // "Terminal" here means THIS task's own run finishing —
                // not a sub-agent's. Sub-agent RunFinished/RunError
                // events are relayed through this same event_tx (via
                // `parent_ctx.relay_event`) so they share the channel,
                // but the parent's drain loop must keep going until ITS
                // own RunFinished arrives. Compare on `event.task_id ==
                // task_id` instead of just matching the variant.
                let is_terminal = matches!(
                    &event.event,
                    AgentEventType::RunFinished { .. } | AgentEventType::RunError { .. }
                ) && event.task_id == task_id;

                // Auto-complete inline hooks
                if let AgentEventType::InlineHookRequested { ref request } = event.event {
                    let _ = executor
                        .complete_inline_hook(&request.hook_id, HookMutation::none())
                        .await;
                }

                // Persist the latest `ContextBudget` on the thread row so the
                // channel `/context` command (and any other non-live surface)
                // can render a breakdown without subscribing to events. Errors
                // are swallowed — bookkeeping must not disrupt the stream.
                if let AgentEventType::ContextBudgetUpdate { ref budget, .. } = event.event {
                    if !event.thread_id.is_empty() {
                        if let Ok(serialized) = serde_json::to_value(budget) {
                            let _ = thread_store
                                .update_last_context_budget(&event.thread_id, Some(serialized))
                                .await;
                        }
                    }
                }

                // Publish to broadcaster keyed on THIS relay's registered
                // task_id (the parent of any relayed sub-agent event), NOT
                // the envelope's `event.task_id`.
                //
                // Why: the dispatch relay loop in `universal_agent.rs`
                // subscribes to the CHILD's broadcaster topic and forwards
                // each child event to the parent's stream via
                // `parent_ctx.relay_event` — which writes the event (with
                // its child task_id intact) into the parent's event_tx.
                // If we then re-publish on `broadcaster[event.task_id]`
                // (= child_task_id), the relay loop's own subscription on
                // that topic re-receives it, re-relays, infinite loop.
                //
                // Publishing on `task_id` (the registered, parent-side
                // key) routes the event to the SSE consumer subscribed
                // to the parent's task — which is where the browser
                // listens — without feeding the loop. The envelope still
                // carries `event.task_id = child_task_id` so consumers
                // route per-task correctly.
                let _ = runtime.broadcaster().publish(&task_id, event).await;

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

    /// After [`apply_agent_overrides`] picks the final (provider, model)
    /// pair, hydrate any empty credentials (`api_key`, `base_url`) by
    /// calling [`ModelSettings::hydrate_creds`] against the workspace
    /// secret store. The same hydration runs for the workspace's
    /// default model in `WorkspaceStore::resolve_model_settings`, so
    /// every ModelSettings handed to the LLM client — whether it came
    /// from the workspace default, an agent's `[model_settings]` pin,
    /// or a runtime override — goes through identical secret
    /// resolution.
    pub async fn hydrate_agent_model_settings(
        &self,
        agent_config: &mut distri_types::configuration::AgentConfig,
    ) -> Result<(), AgentError> {
        let definition = match agent_config {
            distri_types::configuration::AgentConfig::StandardAgent(def) => def,
            distri_types::configuration::AgentConfig::WorkflowAgent(_) => return Ok(()),
        };
        let Some(ref mut ms) = definition.model_settings else {
            return Ok(());
        };
        let Some(secret_store) = self.stores.secret_store.as_ref() else {
            return Ok(());
        };
        ms.hydrate_creds(secret_store.as_ref())
            .await
            .map_err(AgentError::InvalidConfiguration)?;
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
            (Some(agent_model), Some(base)) => match base.merge(&agent_model) {
                Some(m) => Some(m),
                None => {
                    tracing::error!("merge produced empty model for agent '{}'", definition.name,);
                    Some(base.clone())
                }
            },
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
    /// Complete an external tool execution.
    ///
    /// Two paths depending on the `tool_call_id`:
    ///
    ///   - **Workflow wait** — the id is a `wait_task_id` on some
    ///     parked workflow run (the workflow's `ContextEventSink`
    ///     created a child Task for it). Synchronously record the
    ///     result, mark the wait task `Completed`, flip the run task
    ///     `Running`, then **spawn a background re-invocation** of
    ///     the workflow agent on the run task — `run_workflow`'s
    ///     resume detection picks up the stored state and continues
    ///     from the parked frontier.
    ///
    ///   - **Legacy external tool call** — the id is a oneshot
    ///     registered via `ExternalToolCallsStore`. Fire the oneshot.
    ///     (Slated for deletion once external tool calls migrate to
    ///     the wait-task model.)
    pub async fn complete_tool(
        self: Arc<Self>,
        tool_call_id: &str,
        tool_response: distri_types::ToolResponse,
    ) -> Result<(), String> {
        if let Some(workflow_store) = self.workflow_store.clone() {
            if let Ok(Some(task)) = self.stores.task_store.get_task(tool_call_id).await {
                if let Some(parent_id) = task.parent_task_id.clone() {
                    if workflow_store
                        .get_run(&parent_id)
                        .await
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        return self
                            .complete_workflow_wait(
                                &parent_id,
                                tool_call_id,
                                tool_response,
                                workflow_store,
                            )
                            .await;
                    }
                }
            }
        }

        let outcome = self
            .stores
            .external_tool_calls_store
            .complete_external_tool_call(tool_call_id, tool_response)
            .await
            .map_err(|e| e.to_string());
        tracing::info!(
            target: "ext_tool.complete",
            tool_call_id = %tool_call_id,
            ok = outcome.is_ok(),
            error = %outcome.as_ref().err().cloned().unwrap_or_default(),
            "browser POST /complete-tool received"
        );
        outcome
    }

    /// Complete a workflow wait: synchronously record the result on
    /// the step row + flip the wait task `Completed` + flip the run
    /// task `Running`, then spawn the workflow re-drive in the
    /// background. The agent's resume detection hydrates from
    /// `workflow_store` and continues from the parked frontier.
    async fn complete_workflow_wait(
        self: Arc<Self>,
        run_task_id: &str,
        wait_task_id: &str,
        tool_response: distri_types::ToolResponse,
        workflow_store: Arc<dyn distri_workflow::WorkflowStore>,
    ) -> Result<(), String> {
        let steps = workflow_store
            .list_steps(run_task_id)
            .await
            .map_err(|e| e.to_string())?;
        let step = steps
            .into_iter()
            .find(|s| s.wait_task_id.as_deref() == Some(wait_task_id))
            .ok_or_else(|| {
                format!("no step with wait_task_id={wait_task_id} on run {run_task_id}")
            })?;

        let result = serde_json::to_value(&tool_response).map_err(|e| e.to_string())?;
        let now = chrono::Utc::now();
        let updated = distri_workflow::WorkflowStepState {
            step_id: step.step_id.clone(),
            status: distri_types::TaskStatus::Completed,
            result: Some(result),
            error: None,
            started_at: step.started_at,
            completed_at: Some(now),
            wait_task_id: step.wait_task_id.clone(),
        };
        workflow_store
            .upsert_step(run_task_id, updated)
            .await
            .map_err(|e| e.to_string())?;

        self.stores
            .task_store
            .update_task_status(wait_task_id, distri_types::TaskStatus::Completed)
            .await
            .map_err(|e| e.to_string())?;

        self.stores
            .task_store
            .update_task_status(run_task_id, distri_types::TaskStatus::Running)
            .await
            .map_err(|e| e.to_string())?;

        tracing::info!(
            target: "workflow.resume",
            run_task_id = %run_task_id,
            wait_task_id = %wait_task_id,
            step_id = %step.step_id,
            "workflow wait completed; spawning re-drive"
        );

        // Spawn the re-drive — fresh task-local user/workspace
        // context, register_task → spawn_task_relay →
        // spawn_background_execution. The run_workflow path detects
        // the existing WorkflowExecutionState and resumes.
        let self_for_spawn = self.clone();
        let run_task_id_owned = run_task_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = self_for_spawn.resume_workflow_run(run_task_id_owned).await {
                tracing::warn!(error = %e, "workflow auto re-drive failed");
            }
        });

        Ok(())
    }

    /// Spawn a background execution that re-enters a parked workflow
    /// run. Reads `thread_id` / `user_id` / `workspace_id` from the
    /// snapshotted `WorkflowExecutionState`, builds a minimal
    /// `ExecutorContext`, and routes through the standard
    /// `register_task` + `spawn_task_relay` + `spawn_background_execution`
    /// pipeline so the rest of the system sees this exactly like an
    /// A2A `message/send` continuation.
    async fn resume_workflow_run(self: Arc<Self>, run_task_id: String) -> Result<(), String> {
        let workflow_store = self
            .workflow_store
            .as_ref()
            .ok_or("workflow_store not configured")?
            .clone();
        let state = workflow_store
            .get_run(&run_task_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no workflow_run for {run_task_id}"))?;

        let workspace_uuid = state
            .workspace_id
            .as_ref()
            .and_then(|w| uuid::Uuid::parse_str(w).ok());
        let user_id = state.user_id.clone();
        let agent_id = state.agent_id.clone();
        let thread_id = state.thread_id.clone();
        let workspace_str = state.workspace_id.clone();
        let self_inner = self.clone();
        let run_task_id_for_ctx = run_task_id.clone();

        distri_types::context::with_user_and_workspace(user_id.clone(), workspace_uuid, async move {
            let exec_ctx = crate::agent::ExecutorContext {
                thread_id: thread_id.clone(),
                task_id: run_task_id_for_ctx.clone(),
                agent_id: agent_id.clone(),
                user_id: user_id.clone(),
                workspace_id: workspace_str,
                session_id: thread_id.clone(),
                orchestrator: Some(self_inner.clone()),
                ..Default::default()
            };

            let (registered_ctx, event_rx) = self_inner
                .register_task(&run_task_id_for_ctx, &thread_id, exec_ctx)
                .await
                .map_err(|e| format!("register_task: {e}"))?;
            self_inner.spawn_task_relay(run_task_id_for_ctx.clone(), event_rx);
            crate::a2a::stream::spawn_background_execution(
                self_inner.clone(),
                agent_id,
                distri_types::Message::default(),
                registered_ctx,
                None,
                run_task_id_for_ctx,
                user_id,
                workspace_uuid,
            );
            Ok::<(), String>(())
        })
        .await
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
                    tracing::error!("merge produced empty model for LLM call '{}'", llm_def.name);
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

/// Resolve the connections declared in `definition.connections` against the
/// workspace's stored connections, inject resolved env vars into the
/// `ExecutorContext`, and populate `{{available_connections}}` /
/// `{{available_providers}}` dynamic values for the `{{> connections}}`
/// partial.
///
/// Errors out (as `AgentError::Validation`) when a `ConnectionRequirement`
/// marked `required=true` cannot be resolved. Non-required missing
/// requirements are surfaced to the model under `available_providers`.
///
/// A `provider = "*"` requirement is a wildcard: list every connected
/// workspace connection (and available-but-not-connected providers from the
/// registry) without injecting env vars. Used by the top-level distri
/// orchestrator, which authenticates via the `x-connection-id` proxy path.
async fn resolve_declared_connections(
    stores: &distri_types::stores::InitializedStores,
    definition: &distri_types::StandardDefinition,
    context: &Arc<ExecutorContext>,
) -> Result<(), AgentError> {
    use crate::connections::{ConnectionResolver, DefaultResolver, ResolveCtx};
    use distri_types::connections::Connection;

    let conn_store = match stores.connection_store.as_ref() {
        Some(s) => s,
        None => {
            tracing::warn!(
                "Agent '{}' declares connections but no connection_store is configured",
                definition.name
            );
            return Ok(());
        }
    };

    let workspace_id = match &context.workspace_id {
        Some(ws) => ws.clone(),
        None => {
            return Err(AgentError::Validation(format!(
                "Agent '{}' declares connections but context has no workspace_id",
                definition.name
            )));
        }
    };

    // Pre-load workspace connections once so we can match by provider in O(n).
    let ws_connections: Vec<Connection> = conn_store
        .list_by_workspace(&workspace_id)
        .await
        .map_err(|e| AgentError::Validation(format!("failed to list connections: {e}")))?;

    let mut connected_lines: Vec<String> = Vec::new();
    let mut missing_providers: Vec<String> = Vec::new();

    for req in &definition.connections {
        // Wildcard: list every connected workspace connection in the partial
        // but do NOT inject env vars. The agent is expected to authenticate
        // via the `x-connection-id` proxy path (see distri_request /
        // POST /request) rather than raw env-var token reuse. Used by the
        // top-level distri orchestrator agent which manages user connections.
        if req.provider.as_deref() == Some("*") {
            for c in &ws_connections {
                if c.status != distri_types::connections::ConnectionStatus::Connected {
                    continue;
                }
                // Hide the system-seeded DistriNative connection from the
                // listing so the LLM doesn't mistake it for a third-party
                // service to call on the user's behalf.
                if c.auth.is_distri_native() {
                    continue;
                }
                let provider_tag = match &c.auth {
                    distri_types::connections::ConnectionAuth::Oauth {
                        provider, scopes, ..
                    } => format!(", provider: {}, scopes: [{}]", provider, scopes.join(", ")),
                    _ => String::new(),
                };
                connected_lines.push(format!(
                    "- **{}** (id: `{}`, status: connected{})",
                    c.name, c.id, provider_tag
                ));
            }
            continue;
        }

        // Locate the matching connection by id or by provider (matching against
        // the connection's embedded `auth`).
        let matched: Option<Connection> = if let Some(id) = req.connection_id {
            ws_connections.iter().find(|c| c.id == id).cloned()
        } else if let Some(provider) = req.provider.as_deref() {
            let mut found = None;
            for c in ws_connections.iter() {
                let matches_provider = match &c.auth {
                    distri_types::connections::ConnectionAuth::Oauth { provider: p, .. } => {
                        p == provider
                    }
                    distri_types::connections::ConnectionAuth::Custom { .. } => {
                        c.name == provider
                    }
                    distri_types::connections::ConnectionAuth::DistriNative => provider == "distri",
                    distri_types::connections::ConnectionAuth::None => c.name == provider,
                };
                if matches_provider {
                    found = Some(c.clone());
                    break;
                }
            }
            found
        } else {
            return Err(AgentError::Validation(format!(
                "Agent '{}' has a connection requirement with neither provider nor connection_id",
                definition.name
            )));
        };

        let Some(connection) = matched else {
            let label = req
                .provider
                .clone()
                .or_else(|| req.connection_id.map(|id| id.to_string()))
                .unwrap_or_default();
            if req.required {
                return Err(AgentError::Validation(format!(
                    "Agent '{}' requires connection '{}' but none is connected in this workspace",
                    definition.name, label
                )));
            }
            missing_providers.push(format!(
                "- **{}** — not connected yet in this workspace",
                label
            ));
            continue;
        };

        // Scope check (OAuth). Reads the connection's embedded auth directly.
        if !req.scopes.is_empty() {
            if let distri_types::connections::ConnectionAuth::Oauth {
                scopes: granted, ..
            } = &connection.auth
            {
                let missing_scopes: Vec<&String> = req
                    .scopes
                    .iter()
                    .filter(|s| !granted.iter().any(|g| g == *s))
                    .collect();
                if !missing_scopes.is_empty() {
                    if req.required {
                        return Err(AgentError::Validation(format!(
                            "Connection '{}' missing required scopes: {:?}",
                            connection.name, missing_scopes
                        )));
                    }
                    missing_providers.push(format!(
                        "- **{}** — connected but missing scopes: {:?}",
                        connection.name, missing_scopes
                    ));
                    continue;
                }
            }
        }

        // Resolve and merge env vars. The resolver keys off connection_id;
        // auth lives on the connection.
        let mut resolve_ctx = ResolveCtx::new(stores).with_workspace(&workspace_id);
        resolve_ctx = resolve_ctx.with_user(context.user_id.as_str());
        if let Some(ev) = req.env_var.as_deref() {
            resolve_ctx = resolve_ctx.with_env_override(ev);
        }

        match DefaultResolver
            .resolve(&connection.id.to_string(), &resolve_ctx)
            .await
        {
            Ok(resolved) => {
                {
                    let mut env_vars = context.env_vars.write().await;
                    for (k, v) in &resolved.env_vars {
                        env_vars.insert(k.clone(), v.clone());
                    }
                }
                let scopes_tag = match &connection.auth {
                    distri_types::connections::ConnectionAuth::Oauth { scopes, .. } => {
                        format!(", scopes: [{}]", scopes.join(", "))
                    }
                    _ => String::new(),
                };
                connected_lines.push(format!(
                    "- **{}** (id: `{}`, provider: {}{})",
                    connection.name, connection.id, resolved.provider, scopes_tag
                ));
            }
            Err(e) => {
                if req.required {
                    return Err(AgentError::Validation(format!(
                        "Failed to resolve connection '{}': {}",
                        connection.name, e
                    )));
                }
                tracing::warn!(
                    "Agent '{}' declared non-required connection '{}' but resolution failed: {}",
                    definition.name,
                    connection.name,
                    e
                );
                missing_providers.push(format!(
                    "- **{}** — error resolving: {}",
                    connection.name, e
                ));
            }
        }
    }

    // Wildcard path also surfaces registry-known but not-yet-connected providers
    // so the LLM can suggest which ones the user could connect.
    let has_wildcard = definition
        .connections
        .iter()
        .any(|r| r.provider.as_deref() == Some("*"));
    if has_wildcard {
        if let Some(registry) = &stores.provider_registry {
            let connected_names: std::collections::HashSet<String> =
                ws_connections.iter().map(|c| c.name.clone()).collect();
            let all_providers = registry.list_providers().await;
            for provider in &all_providers {
                if !connected_names.contains(provider)
                    && registry.is_provider_available(provider).await
                {
                    missing_providers
                        .push(format!("- **{}** — ready to connect via OAuth", provider));
                }
            }
        }
    }

    let mut dv = std::collections::HashMap::new();
    if !connected_lines.is_empty() {
        dv.insert(
            "available_connections".to_string(),
            serde_json::Value::String(connected_lines.join("\n")),
        );
    }
    if !missing_providers.is_empty() {
        dv.insert(
            "available_providers".to_string(),
            serde_json::Value::String(missing_providers.join("\n")),
        );
    }

    if !dv.is_empty() {
        context
            .merge_hook_prompt_state(crate::agent::context::HookPromptState {
                dynamic_values: dv,
                ..Default::default()
            })
            .await;
    }

    Ok(())
}

/// After a run finishes, warn when an agent declared `connections: [...]` but
/// none of them were resolved (no `inject_connection_env`, no `x-connection-id`,
/// no proxy hit). Helps catch definitions that carry unused connections.
pub async fn warn_unused_connections(
    definition: &distri_types::StandardDefinition,
    context: &Arc<ExecutorContext>,
) {
    if definition.connections.is_empty() {
        return;
    }
    let used = context.connections_used_snapshot().await;
    if !used.is_empty() {
        return;
    }
    let declared: Vec<String> = definition
        .connections
        .iter()
        .map(|r| {
            r.provider
                .clone()
                .or_else(|| r.connection_id.map(|id| id.to_string()))
                .unwrap_or_else(|| "<unspecified>".to_string())
        })
        .collect();
    tracing::warn!(
        event = "connections_declared_but_unused",
        agent = %definition.name,
        declared = ?declared,
        "Agent declared connections but did not use any during this run"
    );
}
