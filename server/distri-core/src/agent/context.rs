use crate::agent::pricing;
use crate::hooks_runtime::HookRegistry;
use distri_stores::SessionStoreExt;
use distri_types::{
    AgentContextSize, AgentPlan, ContextBudget, ContextSize, ContextUsage,
    ExecutionHistoryEntry, ExecutionResult, ModelSettings, Part, PlanStep, RunUsage,
    ScratchpadEntry, ScratchpadEntryType,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::{mpsc, RwLock};

use crate::agent::prompt_registry::PromptSection;
use crate::{
    servers::registry::McpServerRegistry,
    tools::Tool,
    types::Message,
    AgentError, AgentOrchestrator,
};

use super::types::{AgentEvent, AgentEventType};

/// Options for forking ExecutorContext
#[derive(Debug, Clone)]
pub struct ForkOptions {
    pub fork_type: ForkType,
    pub copy_history_limit: Option<usize>,
}

/// Type of fork operation
#[derive(Debug, Clone)]
pub enum ForkType {
    /// Same thread and task, new run ID
    NewRun,
    /// Same thread, new task and run IDs
    NewTask,
    /// New thread, task, and run IDs (complete separation)
    NewThread,
}

pub use distri_types::{AdditionalAttributes, ExecutorContextMetadata};

#[derive(Debug, Clone, Default)]
pub struct PromptTemplateOverride {
    pub system: Option<String>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HookPromptState {
    pub dynamic_sections: Vec<PromptSection>,
    pub dynamic_values: HashMap<String, serde_json::Value>,
    pub template_override: PromptTemplateOverride,
}

impl HookPromptState {
    pub fn merge_from(&mut self, other: HookPromptState) {
        if !other.dynamic_sections.is_empty() {
            self.dynamic_sections = other.dynamic_sections;
        }
        if !other.dynamic_values.is_empty() {
            for (k, v) in other.dynamic_values {
                self.dynamic_values.insert(k, v);
            }
        }

        if other.template_override.system.is_some() {
            self.template_override.system = other.template_override.system;
        }
        if other.template_override.user.is_some() {
            self.template_override.user = other.template_override.user;
        }
    }
}

/// Execution context for agent operations
#[derive(Clone)]
pub struct ExecutorContext {
    pub thread_id: String,
    pub task_id: String, // The user task - multiple runs can belong to same task
    pub run_id: String,  // Individual execution strand - each run has separate history
    pub agent_id: String,
    pub session_id: String,
    pub user_id: String,
    /// Identifier ID for tenant/project-level usage tracking (maps to tenant_id in auth)
    pub identifier_id: Option<String>,
    /// Workspace ID for multi-tenant workspace-scoped tracking
    pub workspace_id: Option<String>,
    /// Channel ID for channel-scoped usage tracking
    pub channel_id: Option<String>,
    /// Tenant context for multi-tenant operations
    /// Stores can use this to filter data by tenant
    pub tenant_context: distri_types::TenantContext,
    /// Browser session ID for browsr - if None, browsr will auto-create one
    pub browser_session_id: Option<String>,
    pub additional_attributes: Option<AdditionalAttributes>,
    // External tools per agent
    pub tools: Arc<RwLock<Vec<Arc<dyn Tool>>>>,
    pub orchestrator: Option<Arc<crate::agent::AgentOrchestrator>>,
    /// Optional context-specific stores (e.g., ephemeral stores for this execution)
    /// If provided, these override the orchestrator's default stores
    pub stores: Option<distri_stores::InitializedStores>,
    pub verbose: bool,
    pub tool_metadata: Option<HashMap<String, serde_json::Value>>,
    pub event_tx: Option<Arc<mpsc::Sender<AgentEvent>>>,
    pub usage: Arc<RwLock<ContextUsage>>,
    pub current_plan: Arc<RwLock<Option<AgentPlan>>>,
    pub task_status: Arc<RwLock<Option<crate::types::TaskStatus>>>,
    pub final_result: Arc<RwLock<Option<Value>>>,
    pub current_step_id: Arc<RwLock<Option<String>>>,
    pub current_message_id: Arc<RwLock<Option<String>>>,
    /// Environment variables passed from the client, forwarded to skill scripts and plugin contexts.
    /// Wrapped in Arc<RwLock> so tools (e.g., inject_connection_env) can mutate env vars
    /// and child contexts inherit the same mutable map.
    pub env_vars: Arc<RwLock<HashMap<String, String>>>,
    /// Channel for emitting events to parent agent (for subagent communication)
    pub parent_tx: Option<Arc<mpsc::Sender<AgentEvent>>>,
    /// Parent task_id for subagents to share session data with parent
    pub parent_task_id: Option<String>,
    /// Outer run_id when this context was spawned by a RemoteAgent.
    /// Set by AgentOrchestrator after looking up the parent run mapping in the broadcaster.
    /// Used by OtelHooks to nest the inner invoke_agent span under the outer one.
    pub parent_run_id: Option<String>,

    pub dynamic_tools: Option<Arc<RwLock<Vec<Arc<dyn Tool>>>>>,
    /// Names of tools that are deferred (name+description only in prompt).
    pub deferred_tool_names: Arc<RwLock<HashSet<String>>>,
    pub hook_prompt_state: Arc<RwLock<HookPromptState>>,
    pub hook_registry: Arc<RwLock<Option<HookRegistry>>>,
    /// Default model settings inherited from the orchestrator/workspace context.
    /// None when no default model is configured yet.
    pub default_model_settings: Option<ModelSettings>,
    /// When true, unsafe tools are simulated via LLM instead of executed.
    /// Safe tools (tool_search, load_skill, final, write_todos) still execute normally.
    pub dry_run: bool,
    /// Runtime mode determines built-in agent tool selection.
    /// Set from metadata at context creation, inherited by child contexts.
    pub runtime_mode: distri_types::RuntimeMode,
    /// LRU cache for file read deduplication (returns FILE_UNCHANGED_STUB for unchanged files)
    pub file_read_cache: Arc<RwLock<distri_types::FileReadCache>>,
    /// Tracks which tool results have been replaced with persisted previews (for prompt cache stability)
    pub content_replacement_state: Arc<RwLock<distri_types::ContentReplacementState>>,
    /// Agent span created by OtelHooks::before_execute; consumed once by StandardAgent::invoke_stream.
    ///
    /// Single-owner contract: only StandardAgent::invoke_stream calls take_otel_agent_span().
    /// Clones share the same Arc, so whichever context calls take() first wins. Do not call
    /// take_otel_agent_span() on inner contexts created by create_inner_context() — those share
    /// this Arc and would silently steal the span from the outer context.
    pub otel_agent_span: Arc<StdMutex<Option<tracing::Span>>>,
    /// Tracks skills loaded inline for re-injection after context compaction
    pub skill_tracker: Arc<RwLock<crate::agent::skill_tracker::ActiveSkillTracker>>,
}

impl std::fmt::Debug for ExecutorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutorContext")
            .field("thread_id", &self.thread_id)
            .field("task_id", &self.task_id)
            .field("run_id", &self.run_id)
            .field("agent_id", &self.agent_id)
            .field("session_id", &self.session_id)
            .field("user_id", &self.user_id)
            .field("verbose", &self.verbose)
            .finish()
    }
}

impl Default for ExecutorContext {
    fn default() -> Self {
        let user_id = uuid::Uuid::new_v4().to_string();
        let workspace_id = None;
        let tenant_context = distri_types::TenantContext::new(user_id.clone(), workspace_id);

        Self {
            thread_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "default".to_string(),
            user_id,
            session_id: uuid::Uuid::new_v4().to_string(),
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
            tenant_context,
            browser_session_id: None,
            tools: Arc::default(),
            orchestrator: None,
            stores: None,
            verbose: false,
            tool_metadata: None,
            event_tx: None,
            usage: Arc::default(),
            current_plan: Arc::new(RwLock::new(None)),
            task_status: Arc::new(RwLock::new(None)),
            final_result: Arc::new(RwLock::new(None)),
            current_step_id: Arc::new(RwLock::new(None)),
            current_message_id: Arc::new(RwLock::new(None)),
            additional_attributes: None,
            env_vars: Arc::new(RwLock::new(HashMap::new())),
            parent_tx: None,
            parent_task_id: None,
            parent_run_id: None,
            dynamic_tools: None,
            deferred_tool_names: Arc::new(RwLock::new(HashSet::new())),
            hook_prompt_state: Arc::new(RwLock::new(HookPromptState::default())),
            hook_registry: Arc::new(RwLock::new(None)),
            default_model_settings: None,
            dry_run: false,
            runtime_mode: distri_types::RuntimeMode::default(),
            file_read_cache: Arc::new(RwLock::new(distri_types::FileReadCache::new(200))),
            content_replacement_state: Arc::new(RwLock::new(
                distri_types::ContentReplacementState::default(),
            )),
            otel_agent_span: Arc::new(StdMutex::new(None)),
            skill_tracker: Arc::new(RwLock::new(
                crate::agent::skill_tracker::ActiveSkillTracker::default(),
            )),
        }
    }
}

impl ExecutorContext {
    pub async fn get_tools(&self) -> Vec<Arc<dyn Tool>> {
        let tools: tokio::sync::RwLockReadGuard<'_, Vec<Arc<dyn Tool>>> = self.tools.read().await;
        tools.clone().into_iter().map(|t| t.clone()).collect()
    }

    pub fn clone_with_tx(&self, event_tx: mpsc::Sender<AgentEvent>) -> Self {
        Self {
            event_tx: Some(Arc::new(event_tx)),
            ..self.clone()
        }
    }

    pub fn set_otel_agent_span(&self, span: tracing::Span) {
        if let Ok(mut guard) = self.otel_agent_span.lock() {
            *guard = Some(span);
        }
    }

    pub fn take_otel_agent_span(&self) -> Option<tracing::Span> {
        self.otel_agent_span.lock().ok()?.take()
    }

    pub async fn clone_with_tools(&self, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self {
            tools: Arc::new(RwLock::new(tools)),
            hook_registry: Arc::new(RwLock::new(self.hook_registry.read().await.clone())),
            ..self.clone()
        }
    }

    pub async fn close_event_tx(&mut self) {
        drop(self.event_tx.take());
    }

    pub async fn extend_tools(&self, tools: Vec<Arc<dyn Tool>>) {
        let mut guard = self.tools.write().await;
        guard.extend(tools);
    }

    /// Set the names of deferred tools (for tool_search awareness).
    pub async fn set_deferred_tool_names(&self, names: HashSet<String>) {
        if !names.is_empty() {
            let mut sorted: Vec<&String> = names.iter().collect();
            sorted.sort();
            self.emit_verbose(format!(
                "[tools] Deferred ({} tools): {}",
                names.len(),
                sorted
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .await;
        }
        let mut guard = self.deferred_tool_names.write().await;
        *guard = names;
    }

    /// Check if a tool is deferred (name+description only, no schema in prompt).
    pub async fn is_tool_deferred(&self, name: &str) -> bool {
        self.deferred_tool_names.read().await.contains(name)
    }

    /// Get a copy of all deferred tool names.
    pub async fn get_deferred_tool_names(&self) -> HashSet<String> {
        self.deferred_tool_names.read().await.clone()
    }

    pub async fn merge_hook_prompt_state(&self, state: HookPromptState) {
        let mut guard = self.hook_prompt_state.write().await;
        guard.merge_from(state);
    }

    pub async fn hook_prompt_state(&self) -> HookPromptState {
        self.hook_prompt_state.read().await.clone()
    }

    pub fn get_orchestrator(&self) -> Result<&Arc<AgentOrchestrator>, AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not intialized".to_string(),
        ))?;
        Ok(orchestrator)
    }

    /// Get the browser session ID if set.
    /// Returns None if no session exists - browsr will auto-create one.
    pub fn get_browser_session_id(&self) -> Option<String> {
        self.browser_session_id.clone()
    }

    /// Set the browser session ID (called after browsr creates a session)
    pub fn set_browser_session_id(&mut self, session_id: Option<String>) {
        self.browser_session_id = session_id;
    }

    pub fn get_registry(&self) -> Result<Arc<RwLock<McpServerRegistry>>, AgentError> {
        let orchestrator = self.get_orchestrator()?;
        let registry = orchestrator.mcp_registry.clone();
        Ok(registry)
    }

    pub async fn emit(&self, event: AgentEventType) {
        // Clone the sender to avoid holding the lock during async sen
        let tx = {
            let guard = self.event_tx.as_ref().cloned();
            guard.as_ref().cloned()
        };

        let event = AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: self.thread_id.clone(),
            run_id: self.run_id.clone(),
            agent_id: self.agent_id.clone(),
            task_id: self.task_id.clone(),
            event,
            user_id: Some(self.user_id.clone()),
            identifier_id: self.identifier_id.clone(),
            workspace_id: self.workspace_id.clone(),
            channel_id: self.channel_id.clone(),
        };

        if let Some(tx) = tx {
            let _ = tx.send(event.clone()).await;
        }

        // Call on_event on system hooks first, then named hooks
        if let Some(orchestrator) = &self.orchestrator {
            for hook in &orchestrator.system_hooks {
                if let Err(e) = hook.on_event(&event).await {
                    tracing::warn!("System hook on_event failed: {}", e);
                }
            }
            let hooks = orchestrator.hooks.read().await;
            for hook in hooks.values() {
                if let Err(e) = hook.on_event(&event).await {
                    tracing::warn!("Hook on_event failed: {}", e);
                }
            }
        }

        // Skip saving artifacts to the task store through events
        // as they are saved separately
        // And text deltas
        if matches!(
            event.event,
            AgentEventType::TextMessageContent { .. } | AgentEventType::BrowserScreenshot { .. }
        ) {
            return;
        }

        if let Some(orchestrator) = &self.orchestrator {
            // Store will use task-local tenant context set at service boundary
            if let Err(e) = orchestrator
                .stores
                .task_store
                .add_event_to_task(&self.task_id, event.clone())
                .await
            {
                tracing::error!("Failed to save event: {}", e);
            }
        }
    }

    /// Emit a verbose diagnostic message to the client (no-op when not verbose).
    /// This is a client-side diagnostic only — server observability comes from
    /// the gen_ai OTel spans (see otel.rs) which flow to pg-traces.
    pub async fn emit_verbose(&self, message: String) {
        if !self.verbose {
            return;
        }
        tracing::debug!("{}", message);
        self.emit(AgentEventType::DiagnosticLog { message }).await;
    }

    /// Emit event to parent agent only (for subagent communication)
    pub async fn emit_parent(&self, event: AgentEventType) {
        let parent_tx = {
            let guard = self.parent_tx.as_ref().cloned();
            guard.as_ref().cloned()
        };

        if let Some(tx) = parent_tx {
            let event = AgentEvent {
                timestamp: chrono::Utc::now(),
                thread_id: self.thread_id.clone(),
                run_id: self.run_id.clone(),
                agent_id: self.agent_id.clone(),
                task_id: self.task_id.clone(),
                event,
                user_id: Some(self.user_id.clone()),
                identifier_id: self.identifier_id.clone(),
                workspace_id: self.workspace_id.clone(),
                channel_id: self.channel_id.clone(),
            };

            let _ = tx.send(event).await;
        }
    }
    pub async fn increment_usage(&self, input_tokens: u32, output_tokens: u32) {
        self.increment_usage_with_cache(input_tokens, output_tokens, 0)
            .await;
    }

    pub async fn increment_usage_with_cache(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        cached_tokens: u32,
    ) {
        let mut usage = self.usage.write().await;
        usage.tokens += input_tokens + output_tokens;
        usage.input_tokens += input_tokens;
        usage.output_tokens += output_tokens;
        usage.cached_tokens += cached_tokens;
    }

    /// Update the context budget breakdown (populated after each prompt build).
    pub async fn update_context_budget(&self, budget: ContextBudget) {
        let mut u = self.usage.write().await;
        u.context_budget = budget;
    }

    /// Set the model name on the usage context for cost tracking
    pub async fn set_usage_model(&self, model: String) {
        let mut usage = self.usage.write().await;
        usage.model = Some(model);
    }

    /// Increment the current iteration count
    pub async fn increment_iteration(&self) {
        let mut usage = self.usage.write().await;
        usage.current_iteration += 1;
    }

    /// Get current usage information
    pub async fn get_usage(&self) -> ContextUsage {
        self.usage.read().await.clone()
    }

    /// Snapshot the current cumulative token counts as the start of a new step.
    /// Must be called at the top of each step iteration before any LLM calls.
    pub async fn snapshot_step_start(&self) {
        let mut u = self.usage.write().await;
        u.step_input_start = u.input_tokens;
        u.step_output_start = u.output_tokens;
        u.step_cached_start = u.cached_tokens;
    }

    /// Returns a `RunUsage` with per-step deltas (tokens used only in this step) and
    /// an estimated cost derived from those deltas.
    pub async fn get_step_usage(&self) -> RunUsage {
        let u = self.usage.read().await;
        let delta_input = u.input_tokens.saturating_sub(u.step_input_start);
        let delta_output = u.output_tokens.saturating_sub(u.step_output_start);
        let delta_cached = u.cached_tokens.saturating_sub(u.step_cached_start);
        let cost = u
            .model
            .as_deref()
            .and_then(|m| pricing::estimate_cost(m, delta_input, delta_output, delta_cached));
        RunUsage {
            total_tokens: delta_input + delta_output,
            input_tokens: delta_input,
            output_tokens: delta_output,
            cached_tokens: delta_cached,
            estimated_tokens: 0,
            model: u.model.clone(),
            cost_usd: cost,
        }
    }

    /// Returns a `RunUsage` with cumulative totals across the entire run plus total cost.
    pub async fn get_total_usage(&self) -> RunUsage {
        let u = self.usage.read().await;
        let cost = u.model.as_deref().and_then(|m| {
            pricing::estimate_cost(m, u.input_tokens, u.output_tokens, u.cached_tokens)
        });
        RunUsage {
            total_tokens: u.tokens,
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cached_tokens: u.cached_tokens,
            estimated_tokens: u.context_size.total_estimated_tokens as u32,
            model: u.model.clone(),
            cost_usd: cost,
        }
    }

    /// Calculate and update context size statistics
    pub async fn calculate_context_size(&self) -> Result<ContextSize, AgentError> {
        use crate::agent::token_estimator::TokenEstimator;
        let mut context_size = ContextSize::default();

        // Calculate message history size and token estimates
        if let Ok(messages) = self.get_message_history().await {
            context_size.message_count = messages.len();
            let message_text: String = messages
                .iter()
                .filter_map(|m| m.as_text())
                .collect::<Vec<_>>()
                .join("\n");

            context_size.message_chars = message_text.len();
            context_size.message_estimated_tokens =
                TokenEstimator::estimate_tokens_max(&message_text);
        }

        // Calculate execution history size and token estimates
        let execution_history = self.get_execution_history().await;
        context_size.execution_history_count = execution_history.len();
        let execution_text: String = execution_history
            .iter()
            .map(|e| e.as_observation())
            .collect::<Vec<_>>()
            .join("\n");

        context_size.execution_history_chars = execution_text.len();
        context_size.execution_history_estimated_tokens =
            TokenEstimator::estimate_tokens_max(&execution_text);

        // Calculate scratchpad size and token estimates
        let scratchpad = self.format_agent_scratchpad(None).await?;
        context_size.scratchpad_chars = scratchpad.len();
        context_size.scratchpad_estimated_tokens = TokenEstimator::estimate_tokens_max(&scratchpad);

        // Calculate per-agent breakdown
        context_size.agent_breakdown = self.calculate_agent_breakdown().await?;

        // Total context size
        context_size.total_chars = context_size.message_chars
            + context_size.execution_history_chars
            + context_size.scratchpad_chars;

        context_size.total_estimated_tokens = context_size.message_estimated_tokens
            + context_size.execution_history_estimated_tokens
            + context_size.scratchpad_estimated_tokens;

        // Store context size in session for persistence
        let session_store = self
            .stores
            .as_ref()
            .and_then(|s| Some(&s.session_store))
            .or_else(|| self.orchestrator.as_ref().map(|o| &o.stores.session_store))
            .ok_or(AgentError::Session("Session store not found".to_string()))?;
        session_store
            .set(
                &self.thread_id,
                &format!("context_size_{}", self.agent_id),
                &context_size,
            )
            .await
            .map_err(|e| {
                AgentError::Session(format!("Failed to set context size in session: {}", e))
            })?;

        // Update the usage stats
        {
            let mut usage = self.usage.write().await;
            usage.context_size = context_size.clone();
        }

        Ok(context_size)
    }

    /// Calculate context size breakdown by agent (based on task_id)
    pub async fn calculate_agent_breakdown(
        &self,
    ) -> Result<HashMap<String, AgentContextSize>, AgentError> {
        let mut agent_breakdown = HashMap::new();

        if let Some(orchestrator) = &self.orchestrator {
            // Get all scratchpad entries to analyze by task_id
            // Clone the Arc to keep stores alive during the async operation
            let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
            if let Ok(entries) = scratchpad_store
                .get_entries(&self.thread_id, &self.task_id, None)
                .await
            {
                let mut task_id_to_agent: HashMap<String, String> = HashMap::new();

                // Group entries by task_id and calculate per-agent stats
                for entry in entries {
                    if let crate::agent::context::ScratchpadEntryType::Execution(exec_entry) =
                        &entry.entry_type
                    {
                        let task_id = &exec_entry.task_id;

                        // Try to determine agent_id from context - for now use task_id prefix or fallback
                        let agent_id =
                            task_id_to_agent.get(task_id).cloned().unwrap_or_else(|| {
                                // Try to infer agent from context or use current agent
                                if task_id == &self.task_id {
                                    self.agent_id.clone()
                                } else {
                                    format!("agent-{}", &task_id[..std::cmp::min(8, task_id.len())])
                                }
                            });

                        task_id_to_agent.insert(task_id.clone(), agent_id.clone());

                        let agent_stats =
                            agent_breakdown.entry(agent_id.clone()).or_insert_with(|| {
                                AgentContextSize {
                                    agent_id: agent_id.clone(),
                                    task_count: 0,
                                    execution_history_count: 0,
                                    execution_history_chars: 0,
                                    execution_history_estimated_tokens: 0,
                                    scratchpad_chars: 0,
                                    scratchpad_estimated_tokens: 0,
                                }
                            });

                        let observation = exec_entry.execution_result.as_observation();
                        agent_stats.execution_history_count += 1;
                        agent_stats.execution_history_chars += observation.len();
                        agent_stats.execution_history_estimated_tokens +=
                            crate::agent::token_estimator::TokenEstimator::estimate_tokens_max(
                                &observation,
                            );
                    }
                }

                // Count unique tasks per agent
                for (_, agent_id) in task_id_to_agent {
                    if let Some(stats) = agent_breakdown.get_mut(&agent_id) {
                        stats.task_count += 1;
                    }
                }

                // Calculate scratchpad size per agent
                for (agent_id, stats) in agent_breakdown.iter_mut() {
                    // Get scratchpad for this agent's tasks (simplified - use current agent's scratchpad for now)
                    if agent_id == &self.agent_id {
                        let scratchpad = self.format_agent_scratchpad(None).await?;
                        stats.scratchpad_chars = scratchpad.len();
                        stats.scratchpad_estimated_tokens =
                            crate::agent::token_estimator::TokenEstimator::estimate_tokens_max(
                                &scratchpad,
                            );
                    }
                }
            }
        }

        Ok(agent_breakdown)
    }

    /// Get current context size without recalculating
    pub async fn get_context_size(&self) -> ContextSize {
        let usage = self.usage.read().await;
        usage.context_size.clone()
    }

    /// Get context size from session storage (persisted)
    pub async fn get_context_size_from_session(&self) -> Result<Option<ContextSize>, AgentError> {
        let session_store = self
            .stores
            .as_ref()
            .and_then(|s| Some(&s.session_store))
            .or_else(|| self.orchestrator.as_ref().map(|o| &o.stores.session_store))
            .ok_or(AgentError::Session("Session store not found".to_string()))?;
        let session_key = format!("context_size_{}", self.agent_id);
        let val = session_store
            .get_value(&self.thread_id, &session_key)
            .await
            .map_err(|e| {
                AgentError::Session(format!("Failed to get context size from session: {}", e))
            })?;
        val.map(|v| {
            serde_json::from_value(v).map_err(|e| {
                AgentError::Parsing(format!("Failed to parse context size from session: {}", e))
            })
        })
        .transpose()
    }

    /// Get current iteration information as a formatted string
    pub async fn get_iteration_info(&self, max_iterations: usize) -> String {
        let usage = self.usage.read().await;
        format!("{}/{}", usage.current_iteration, max_iterations)
    }

    /// Convenience method to add a message to the task store
    /// Note: Caller must ensure task-local tenant context is set at service boundary
    pub async fn save_message(&self, message: &Message) {
        if let Some(orchestrator) = &self.orchestrator {
            tracing::debug!(
                "Saving message to thread_id: {}, task_id: {}, user_id: {}, workspace_id: {:?}",
                self.thread_id,
                self.task_id,
                self.user_id,
                self.workspace_id
            );

            if let Err(e) = orchestrator
                .stores
                .task_store
                .add_message_to_task(&self.task_id, message)
                .await
            {
                tracing::error!("Failed to save message: {}", e);
            } else {
                tracing::debug!(
                    "Successfully saved message to thread_id: {}, task_id: {}",
                    self.thread_id,
                    self.task_id
                );
            }
        } else {
            tracing::error!("No orchestrator found - cannot save message. thread_id: {}, task_id: {}, run_id: {}",
                          self.thread_id, self.task_id, self.run_id);
        }
    }

    /// Update task status and emit event with latest state
    pub async fn update_status(&self, status: crate::types::TaskStatus) {
        if let Some(orchestrator) = &self.orchestrator {
            tracing::debug!(
                "Updating task status to {:?} for task_id: {}",
                status,
                self.task_id
            );

            // Update the task status in the store
            if let Err(e) = orchestrator
                .update_task_status(&self.task_id, status.clone())
                .await
            {
                tracing::error!("Failed to update task status to {:?}: {}", status, e);
                return;
            }

            tracing::debug!(
                "Successfully updated task status to {:?} for task_id: {}",
                status,
                self.task_id
            );
            {
                let mut guard = self.task_status.write().await;
                *guard = Some(status);
            };
        } else {
            tracing::error!("No orchestrator found - cannot update task status. thread_id: {}, task_id: {}, run_id: {}", 
                          self.thread_id, self.task_id, self.run_id);
        }
    }

    pub async fn get_status(&self) -> Option<crate::types::TaskStatus> {
        self.task_status.read().await.clone()
    }

    pub async fn set_final_result(&self, result: Option<Value>) {
        let mut guard = self.final_result.write().await;
        *guard = result;
    }

    pub async fn get_final_result(&self) -> Option<Value> {
        self.final_result.read().await.clone()
    }

    /// Set the current plan
    pub async fn set_current_plan(&self, plan: Option<AgentPlan>) {
        let mut guard = self.current_plan.write().await;
        *guard = plan;
    }

    /// Get the current plan
    pub async fn get_current_plan(&self) -> Option<AgentPlan> {
        self.current_plan.read().await.clone()
    }

    /// Get message history directly from task store
    pub async fn get_message_history(
        &self,
    ) -> Result<Vec<crate::types::Message>, crate::AgentError> {
        self.collect_message_history(None).await
    }

    /// Get message history for the current task/run only
    pub async fn get_current_task_message_history(
        &self,
    ) -> Result<Vec<crate::types::Message>, crate::AgentError> {
        self.collect_message_history(Some(&self.task_id)).await
    }

    async fn collect_message_history(
        &self,
        task_filter: Option<&str>,
    ) -> Result<Vec<crate::types::Message>, crate::AgentError> {
        tracing::debug!(
            "Getting message history for thread_id: {} (task filter: {:?})",
            self.thread_id,
            task_filter
        );

        if let Some(orchestrator) = &self.orchestrator {
            // Store will use task-local tenant context set at service boundary
            if let Ok(task_history) = orchestrator
                .stores
                .task_store
                .get_history(&self.thread_id, None)
                .await
            {
                tracing::debug!(
                    "Retrieved task history with {} entries for thread_id: {}",
                    task_history.len(),
                    self.thread_id
                );

                let mut messages = Vec::new();
                for (task, task_messages) in &task_history {
                    if let Some(filter_task_id) = task_filter {
                        if task.id != filter_task_id {
                            continue;
                        }
                    }
                    tracing::debug!(
                        "Processing task {} with {} messages",
                        task.id,
                        task_messages.len()
                    );
                    for task_message in task_messages {
                        if let crate::types::TaskMessage::Message(message) = task_message {
                            tracing::debug!("Found message: {:?}", message);
                            messages.push(message.clone());
                        }
                    }
                }

                messages.sort_by_key(|m| m.created_at);
                tracing::debug!(
                    "Returning {} messages for thread_id: {} (task filter: {:?})",
                    messages.len(),
                    self.thread_id,
                    task_filter
                );
                return Ok(messages);
            } else {
                tracing::warn!(
                    "Failed to get history from task store for thread_id: {}",
                    self.thread_id
                );
            }
        } else {
            tracing::error!(
                "No orchestrator found when getting message history for thread_id: {}",
                self.thread_id
            );
        }
        Ok(Vec::new())
    }

    /// Create a new task context within the same conversation thread.
    /// Child gets its own usage counter to avoid double-counting tokens
    /// when both parent and child emit RunFinished events.
    pub async fn new_task(&self, agent_id: &str) -> Self {
        ExecutorContext {
            thread_id: self.thread_id.clone(),
            agent_id: agent_id.to_string(),
            session_id: self.session_id.clone(),
            user_id: self.user_id.clone(),
            identifier_id: self.identifier_id.clone(),
            workspace_id: self.workspace_id.clone(),
            channel_id: self.channel_id.clone(),
            tenant_context: self.tenant_context.clone(),
            // Child gets its own usage counter — NOT shared with parent.
            // This prevents double-counting: child records its own tokens via RunFinished,
            // and parent records only its own LLM calls.
            verbose: self.verbose,
            orchestrator: self.orchestrator.clone(),
            stores: self
                .stores
                .clone()
                .or_else(|| self.orchestrator.as_ref().map(|o| o.stores.clone())),
            dynamic_tools: self.dynamic_tools.clone(),
            event_tx: self.event_tx.clone(),
            parent_task_id: Some(self.task_id.clone()),
            tool_metadata: self.tool_metadata.clone(),
            default_model_settings: self.default_model_settings.clone(),
            env_vars: self.env_vars.clone(),
            runtime_mode: self.runtime_mode.clone(),
            skill_tracker: Arc::new(RwLock::new(self.skill_tracker.read().await.clone())),

            ..Default::default()
        }
    }

    /// Create a continuation context for agent handover (transfer_to_agent).
    /// Unlike new_task(), this preserves the SAME task_id and scratchpad so the
    /// target agent can see everything the parent already did. Only the agent_id
    /// and run_id change.
    pub async fn continue_as(&self, agent_id: &str) -> Self {
        ExecutorContext {
            thread_id: self.thread_id.clone(),
            agent_id: agent_id.to_string(),
            task_id: self.task_id.clone(), // SAME task — shared history
            run_id: uuid::Uuid::new_v4().to_string(), // new run for the target agent
            session_id: self.session_id.clone(),
            user_id: self.user_id.clone(),
            identifier_id: self.identifier_id.clone(),
            workspace_id: self.workspace_id.clone(),
            channel_id: self.channel_id.clone(),
            tenant_context: self.tenant_context.clone(),
            usage: self.usage.clone(),
            verbose: self.verbose,
            orchestrator: self.orchestrator.clone(),
            stores: self
                .stores
                .clone()
                .or_else(|| self.orchestrator.as_ref().map(|o| o.stores.clone())),
            dynamic_tools: self.dynamic_tools.clone(),
            event_tx: self.event_tx.clone(),
            parent_task_id: self.parent_task_id.clone(),
            tool_metadata: self.tool_metadata.clone(),
            default_model_settings: self.default_model_settings.clone(),
            env_vars: self.env_vars.clone(),
            runtime_mode: self.runtime_mode.clone(),

            ..Default::default()
        }
    }

    /// Fork the context for branching/compaction
    /// This allows copying selective history and creating new execution strands
    pub async fn fork(&self, options: ForkOptions) -> Self {
        let mut forked_context = self.clone();

        // Create new IDs based on fork type
        match options.fork_type {
            ForkType::NewRun => {
                // Same thread and task, new run
                forked_context.run_id = uuid::Uuid::new_v4().to_string();
            }
            ForkType::NewTask => {
                // Same thread, new task and run
                forked_context.task_id = uuid::Uuid::new_v4().to_string();
                forked_context.run_id = uuid::Uuid::new_v4().to_string();
            }
            ForkType::NewThread => {
                // Completely new thread, task, and run
                forked_context.thread_id = uuid::Uuid::new_v4().to_string();
                forked_context.task_id = uuid::Uuid::new_v4().to_string();
                forked_context.run_id = uuid::Uuid::new_v4().to_string();
            }
        }

        // History is managed in stores, not in context
        // copy_history_limit option can be handled by individual store implementations if needed
        let _ = options.copy_history_limit; // Acknowledge the option but don't use it here

        // Reset volatile state
        {
            let mut usage_guard = forked_context.usage.write().await;
            *usage_guard = ContextUsage::default();
        }
        forked_context.hook_registry = Arc::new(RwLock::new(None));

        forked_context
    }

    /// Create an inner context for stream processing with event forwarding
    /// This creates a new context with a new event channel that can be used for forwarding
    pub async fn create_inner_context(&self) -> (Self, mpsc::Receiver<AgentEvent>) {
        use tokio::sync::mpsc;

        // Create a new event channel for the inner context
        let (inner_tx, inner_rx) = mpsc::channel(10000);

        // Create a completely new ExecutorContext instance with copied attributes
        let inner_context = ExecutorContext {
            thread_id: self.thread_id.clone(),
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(), // Keep the same run_id for consistency
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            user_id: self.user_id.clone(),
            identifier_id: self.identifier_id.clone(),
            workspace_id: self.workspace_id.clone(),
            channel_id: self.channel_id.clone(),
            tenant_context: self.tenant_context.clone(),
            browser_session_id: self.browser_session_id.clone(),
            tools: self.tools.clone(),               // Arc::clone
            orchestrator: self.orchestrator.clone(), // Arc::clone
            stores: self
                .stores
                .clone()
                .or_else(|| self.orchestrator.as_ref().map(|o| o.stores.clone())),
            verbose: self.verbose,
            tool_metadata: self.tool_metadata.clone(),
            event_tx: Some(Arc::new(inner_tx)), // New event channel
            usage: self.usage.clone(),          // New usage tracking
            current_plan: self.current_plan.clone(), // Arc::clone
            task_status: self.task_status.clone(), // Arc::clone
            final_result: self.final_result.clone(), // Arc::clone
            current_step_id: self.current_step_id.clone(), // Arc::clone
            current_message_id: self.current_message_id.clone(), // Arc::clone
            additional_attributes: self.additional_attributes.clone(),
            env_vars: self.env_vars.clone(),

            parent_tx: self.parent_tx.clone(),
            parent_task_id: self.parent_task_id.clone(),
            parent_run_id: self.parent_run_id.clone(),
            dynamic_tools: self.dynamic_tools.clone(),
            deferred_tool_names: self.deferred_tool_names.clone(),
            hook_prompt_state: self.hook_prompt_state.clone(),
            hook_registry: self.hook_registry.clone(),
            default_model_settings: self.default_model_settings.clone(),
            dry_run: self.dry_run,
            runtime_mode: self.runtime_mode.clone(),
            file_read_cache: self.file_read_cache.clone(),
            content_replacement_state: self.content_replacement_state.clone(),
            otel_agent_span: self.otel_agent_span.clone(),
            skill_tracker: self.skill_tracker.clone(),
        };

        (inner_context, inner_rx)
    }

    /// Get execution history from scratchpad store with optional task_id filter
    pub async fn get_execution_history(&self) -> Vec<ExecutionResult> {
        if let Some(orchestrator) = &self.orchestrator {
            // Clone the Arc to keep stores alive during the async operation
            let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
            let entries = scratchpad_store
                .get_entries(&self.thread_id, &self.task_id, None)
                .await
                .unwrap_or_default();

            return entries
                .into_iter()
                .filter_map(|entry| match entry.entry_type {
                    ScratchpadEntryType::Execution(exec_entry) => Some(exec_entry.execution_result),
                    _ => None,
                })
                .collect();
        }
        Vec::new()
    }

    /// Store plan step in scratchpad store
    pub async fn store_plan_step(&self, step: &PlanStep) {
        if let Some(orchestrator) = &self.orchestrator {
            let entry = ScratchpadEntry {
                timestamp: chrono::Utc::now().timestamp_millis(),
                entry_type: ScratchpadEntryType::PlanStep(step.clone()),
                task_id: self.task_id.clone(),
                parent_task_id: self.parent_task_id.clone(),
                entry_kind: Some("plan_step".to_string()),
            };
            // Clone the Arc to keep stores alive during the async operation
            let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
            let _ = scratchpad_store.add_entry(&self.thread_id, entry).await;
        }
    }

    pub async fn store_task(&self, task: &Vec<Part>) {
        if let Some(orchestrator) = &self.orchestrator {
            let entry = ScratchpadEntry {
                timestamp: chrono::Utc::now().timestamp_millis(),
                entry_type: ScratchpadEntryType::Task(task.clone()),
                task_id: self.task_id.clone(),
                parent_task_id: self.parent_task_id.clone(),
                entry_kind: Some("task".to_string()),
            };
            // Clone the Arc to keep stores alive during the async operation
            let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
            let _ = scratchpad_store.add_entry(&self.thread_id, entry).await;
        }
    }

    /// Store execution result compacted in the scratchpad.
    pub async fn store_execution_result(&self, result: &ExecutionResult) -> Result<(), AgentError> {
        tracing::debug!("Storing execution result for task_id: {}", self.task_id);

        let compacted = result.compact_for_storage();
        let exec_entry = ExecutionHistoryEntry {
            thread_id: self.thread_id.clone(),
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            execution_result: compacted,
            stored_at: chrono::Utc::now().timestamp_millis(),
        };

        let entry = ScratchpadEntry {
            timestamp: exec_entry.stored_at,
            entry_type: ScratchpadEntryType::Execution(exec_entry),
            task_id: self.task_id.clone(),
            parent_task_id: self.parent_task_id.clone(),
            entry_kind: Some("execution".to_string()),
        };
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not intialized".to_string(),
        ))?;

        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        scratchpad_store.add_entry(&self.thread_id, entry).await?;
        Ok(())
    }

    /// Check file read cache and return FILE_UNCHANGED_STUB if file hasn't changed.
    ///
    /// Returns `Some(FILE_UNCHANGED_STUB)` if the file is unchanged,
    /// `None` if the file needs to be re-read (cache miss or changed).
    pub async fn check_file_read_cache(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        mtime_ns: Option<i64>,
    ) -> Option<&'static str> {
        let cache = self.file_read_cache.read().await;
        match cache.check(path, offset, limit, mtime_ns) {
            distri_types::CacheCheck::Unchanged => {
                tracing::debug!("File read cache hit (unchanged): {}", path);
                Some(distri_types::FILE_UNCHANGED_STUB)
            }
            _ => None,
        }
    }

    /// Record a file read in the cache (called after successful read).
    pub async fn record_file_read(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        content: &str,
        mtime_ns: Option<i64>,
    ) {
        let mut cache = self.file_read_cache.write().await;
        cache.record(path, offset, limit, content, mtime_ns);
    }

    /// Invalidate file read cache for a path (called after file edits).
    pub async fn invalidate_file_cache(&self, path: &str) {
        let mut cache = self.file_read_cache.write().await;
        cache.invalidate(path);
    }

    /// Format agent scratchpad using scratchpad store with context size management
    pub async fn format_agent_scratchpad(
        &self,
        limit: Option<usize>,
    ) -> Result<String, AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not intialized".to_string(),
        ))?;

        // Clone the Arc to keep stores alive during the async operation
        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        let entries = scratchpad_store
            .get_entries(&self.thread_id, &self.task_id, limit)
            .await
            .unwrap_or_default();

        // Use ContextSizeManager to trim entries based on token count
        let max_tokens = self.get_scratchpad_token_limit();
        let context_manager =
            crate::agent::context_size_manager::ContextSizeManager::with_max_tokens(max_tokens);
        let trimmed_entries = context_manager.trim_scratchpad_entries(&entries);

        return Ok(
            crate::agent::strategy::planning::scratchpad::format_scratchpad_with_task_filter(
                &trimmed_entries,
                None, // Don't apply additional limit since we already trimmed
                Some(&self.task_id),
            ),
        );
    }

    /// Get appropriate token limit for scratchpad based on model capabilities
    fn get_scratchpad_token_limit(&self) -> usize {
        // TODO Make this configurable based on model type
        // For now, use conservative defaults that work for most models
        8000
    }

    /// Get/set current message/step ID - temporary no-ops for compatibility
    pub async fn get_current_message_id(&self) -> Option<String> {
        self.current_message_id.read().await.clone()
    }
    pub async fn set_current_message_id(&self, id: Option<String>) {
        let mut guard = self.current_message_id.write().await;
        *guard = id;
    }
    pub async fn get_current_step_id(&self) -> Option<String> {
        self.current_step_id.read().await.clone()
    }
    pub async fn set_current_step_id(&self, id: Option<String>) {
        let mut guard = self.current_step_id.write().await;
        *guard = id;
    }

    /// Evaluate context size and perform compaction if needed.
    ///
    /// This should be called before each LLM call in the agent loop.
    /// It checks the current context usage ratio against configured thresholds
    /// and applies the appropriate compaction tier:
    /// - Tier 1 (Trim): Mechanical truncation of old entries
    /// - Tier 2 (Summarize): Signals that LLM summarization is needed
    /// - Tier 3 (Reset): Emergency - keeps only task + last 2 entries
    ///
    /// Emits a `ContextCompaction` event when compaction occurs.
    /// Returns the compaction result for the caller to act on (e.g., perform LLM summarization).
    pub async fn evaluate_compaction(
        &self,
    ) -> Result<Option<crate::agent::context_size_manager::CompactionResult>, AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not initialized".to_string(),
        ))?;

        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        let entries = scratchpad_store
            .get_entries(&self.thread_id, &self.task_id, None)
            .await
            .unwrap_or_default();

        if entries.is_empty() {
            return Ok(None);
        }

        let max_tokens = self.get_scratchpad_token_limit();
        let manager =
            crate::agent::context_size_manager::ContextSizeManager::with_max_tokens(max_tokens);
        let result = manager.evaluate_and_compact(&entries);

        // If compaction was applied, re-inject skills and emit the event
        if let Some(ref tier) = result.tier {
            // Re-inject tracked skill content after compaction
            let reinjected_skills = match self.reinject_skills().await {
                Ok(ids) => {
                    if !ids.is_empty() {
                        tracing::info!(
                            "Re-injected {} skills after {:?} compaction: {:?}",
                            ids.len(),
                            tier,
                            ids
                        );
                    }
                    ids
                }
                Err(e) => {
                    tracing::warn!("Failed to re-inject skills after compaction: {}", e);
                    vec![]
                }
            };

            self.emit(AgentEventType::ContextCompaction {
                tier: tier.clone(),
                tokens_before: result.tokens_before,
                tokens_after: result.tokens_after,
                entries_affected: result.entries_affected,
                context_limit: max_tokens,
                usage_ratio: result.usage_ratio,
                summary: None,
                reinjected_skills,
                context_budget: Some(self.get_usage().await.context_budget.clone()),
            })
            .await;

            tracing::info!(
                "Context compaction ({:?}): {} → {} tokens, {} entries affected, {:.0}% usage",
                tier,
                result.tokens_before,
                result.tokens_after,
                result.entries_affected,
                result.usage_ratio * 100.0,
            );

            return Ok(Some(result));
        }

        Ok(None)
    }

    /// Emit a `ContextBudgetUpdate` event if utilization has changed significantly.
    ///
    /// Rate-limited: only emits when utilization changes by >5% since last emission,
    /// or when crossing warning (80%) or critical (90%) thresholds.
    pub async fn emit_budget_update(&self) {
        let budget = self.get_usage().await.context_budget.clone();
        let utilization = budget.utilization();
        let is_warning = budget.is_warning();
        let is_critical = budget.is_critical();

        // Rate-limit: only emit if utilization is meaningful (>0) and we have a window
        if budget.context_window_size == 0 {
            return;
        }

        // Always emit when crossing warning/critical thresholds, or on significant change
        // The printer tracks the last state, so duplicates are cheap
        self.emit(AgentEventType::ContextBudgetUpdate {
            budget,
            is_warning,
            is_critical,
        })
        .await;

        tracing::debug!(
            "Context budget update: {:.0}% utilization (warning={}, critical={})",
            utilization * 100.0,
            is_warning,
            is_critical,
        );
    }

    /// Store a compaction summary in the scratchpad.
    /// Called after Tier 2 LLM summarization produces a summary text.
    pub async fn store_compaction_summary(
        &self,
        summary: distri_types::CompactionSummary,
    ) -> Result<(), AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not initialized".to_string(),
        ))?;

        let entry = ScratchpadEntry {
            timestamp: chrono::Utc::now().timestamp_millis(),
            entry_type: ScratchpadEntryType::Summary(summary),
            task_id: self.task_id.clone(),
            parent_task_id: self.parent_task_id.clone(),
            entry_kind: Some("summary".to_string()),
        };

        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        scratchpad_store.add_entry(&self.thread_id, entry).await?;
        Ok(())
    }

    /// Re-inject tracked skill content as SkillContext scratchpad entries.
    /// Called after compaction to ensure skill content survives.
    /// Returns the list of skill IDs that were re-injected.
    pub async fn reinject_skills(&self) -> Result<Vec<String>, AgentError> {
        let tracker = self.skill_tracker.read().await;
        let candidates = tracker.get_reinjection_candidates();

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not initialized".to_string(),
        ))?;
        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();

        let mut reinjected_ids = vec![];
        let now = chrono::Utc::now().timestamp_millis();

        for candidate in &candidates {
            let entry = distri_types::ScratchpadEntry {
                timestamp: now,
                entry_type: distri_types::ScratchpadEntryType::SkillContext(
                    distri_types::SkillContextEntry {
                        skill_id: candidate.skill_id.clone(),
                        content: candidate.content.clone(),
                        reinjected_at: now,
                    },
                ),
                task_id: self.task_id.clone(),
                parent_task_id: self.parent_task_id.clone(),
                entry_kind: Some("skill_context".to_string()),
            };

            scratchpad_store.add_entry(&self.thread_id, entry).await?;

            reinjected_ids.push(candidate.skill_id.clone());
        }

        Ok(reinjected_ids)
    }

    /// Store a CompactionSummary as a scratchpad entry
    pub async fn store_summary_entry(
        &self,
        summary: &distri_types::CompactionSummary,
    ) -> Result<(), AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not initialized".to_string(),
        ))?;
        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();

        let entry = distri_types::ScratchpadEntry {
            timestamp: chrono::Utc::now().timestamp_millis(),
            entry_type: distri_types::ScratchpadEntryType::Summary(summary.clone()),
            task_id: self.task_id.clone(),
            parent_task_id: self.parent_task_id.clone(),
            entry_kind: Some("summary".to_string()),
        };

        scratchpad_store.add_entry(&self.thread_id, entry).await?;
        Ok(())
    }

    /// Get raw scratchpad entries for the current task
    pub async fn get_scratchpad_entries(
        &self,
    ) -> Result<Vec<distri_types::ScratchpadEntry>, AgentError> {
        let orchestrator = self.orchestrator.as_ref().ok_or(AgentError::Execution(
            "Orchestrator not initialized".to_string(),
        ))?;
        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        let entries = scratchpad_store
            .get_entries(&self.thread_id, &self.task_id, None)
            .await
            .unwrap_or_default();
        Ok(entries)
    }

    /// Set agent_id - for context switching in multi-agent scenarios
    pub async fn set_agent_id(&mut self, agent_id: String) {
        self.agent_id = agent_id;
    }

    pub fn get_session_store(
        &self,
    ) -> Result<&Arc<dyn distri_types::stores::SessionStore>, AgentError> {
        self.stores
            .as_ref()
            .and_then(|s| Some(&s.session_store))
            .or_else(|| self.orchestrator.as_ref().map(|o| &o.stores.session_store))
            .ok_or(AgentError::Session("Session store not found".to_string()))
    }
}

#[cfg(test)]
impl ExecutorContext {
    /// Minimal context for unit testing tools in isolation.
    pub fn new_minimal_for_test(stores: distri_stores::InitializedStores) -> Self {
        Self {
            stores: Some(stores),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod otel_tests {
    use super::*;
    #[test]
    fn otel_agent_span_round_trip() {
        let ctx = ExecutorContext::default();
        assert!(ctx.take_otel_agent_span().is_none(), "initially None");
        let span = tracing::info_span!("test");
        ctx.set_otel_agent_span(span);
        let taken = ctx.take_otel_agent_span();
        assert!(taken.is_some(), "should be Some after set");
        assert!(ctx.take_otel_agent_span().is_none(), "None after take");
    }
}
