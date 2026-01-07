use crate::hooks_runtime::HookRegistry;
use distri_stores::SessionStoreExt;
use distri_types::{
    auth::AuthSession, configuration::DefinitionOverrides, AgentContextSize, AgentPlan,
    ContextSize, ContextUsage, ExecutionHistoryEntry, ExecutionResult, Part, PlanStep,
    ScratchpadEntry, ScratchpadEntryType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

use crate::agent::prompt_registry::PromptSection;
use crate::{
    servers::registry::McpServerRegistry,
    tools::Tool,
    types::{ExternalTool, Message},
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutorContextMetadata {
    /// Add additional context for tools to use passed as meta in tool calls
    pub tool_metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub additional_attributes: Option<AdditionalAttributes>,

    /// Define additional tools to delegate to during execution
    pub external_tools: Option<Vec<ExternalTool>>,

    /// Optional definition overrides supplied by the client
    #[serde(default)]
    pub definition_overrides: Option<DefinitionOverrides>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdditionalAttributes {
    pub thread: Option<serde_json::Value>,
    pub task: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserSession {
    pub browser_session_id: Option<String>,
    pub sequence_id: Option<String>,
}

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
    pub auth_session: Option<AuthSession>,
    /// Channel for emitting events to parent agent (for subagent communication)
    pub parent_tx: Option<Arc<mpsc::Sender<AgentEvent>>>,
    /// Parent task_id for subagents to share session data with parent
    pub parent_task_id: Option<String>,

    pub dynamic_tools: Option<Arc<RwLock<Vec<Arc<dyn Tool>>>>>,
    pub hook_prompt_state: Arc<RwLock<HookPromptState>>,
    pub hook_registry: Arc<RwLock<Option<HookRegistry>>>,
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
        Self {
            thread_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "default".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
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
            auth_session: None,
            parent_tx: None,
            parent_task_id: None,
            dynamic_tools: None,
            hook_prompt_state: Arc::new(RwLock::new(HookPromptState::default())),
            hook_registry: Arc::new(RwLock::new(None)),
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

    /// Resolve browser session and sequence identifiers consistently from additional attributes,
    /// falling back to the executor context identifiers when not provided.
    pub fn browser_session_ids(&self) -> (String, String) {
        let session = self
            .additional_attributes
            .as_ref()
            .and_then(|a| a.thread.as_ref())
            .and_then(|v| serde_json::from_value::<BrowserSession>(v.clone()).ok());

        let session_id = session
            .as_ref()
            .and_then(|s| s.browser_session_id.clone())
            .unwrap_or_else(|| self.session_id.clone());

        let sequence_id = session
            .and_then(|s| s.sequence_id)
            .unwrap_or_else(|| self.task_id.clone());

        (session_id, sequence_id)
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
        };

        if let Some(tx) = tx {
            let _ = tx.send(event.clone()).await;
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
            // Clone the Arc to keep stores alive during the async operation
            let task_store = orchestrator.stores.task_store.clone();
            if let Err(e) = task_store
                .add_event_to_task(&self.task_id, event) // Use thread_id for conversation events
                .await
            {
                tracing::error!("Failed to save event: {}", e);
            }
        }
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
            };

            let _ = tx.send(event).await;
        }
    }
    pub async fn increment_usage(&self, input_tokens: u32, output_tokens: u32) {
        let mut usage = self.usage.write().await;
        usage.tokens += input_tokens + output_tokens;
        usage.input_tokens += input_tokens;
        usage.output_tokens += output_tokens;
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
    pub async fn save_message(&self, message: &Message) {
        if let Some(orchestrator) = &self.orchestrator {
            tracing::debug!(
                "Saving message to thread_id: {}, message: {:?}",
                self.thread_id,
                message
            );
            if let Err(e) = orchestrator
                .stores
                .task_store
                .add_message_to_task(&self.task_id, message) // Use task_id to store messages in the correct task
                .await
            {
                tracing::error!("Failed to save message: {}", e);
            } else {
                tracing::debug!(
                    "Successfully saved message to thread_id: {}",
                    self.thread_id
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

    /// Create a new task context within the same conversation thread
    pub async fn new_task(&self, agent_id: &str) -> Self {
        ExecutorContext {
            thread_id: self.thread_id.clone(),
            agent_id: agent_id.to_string(),
            session_id: self.session_id.clone(),
            user_id: self.user_id.clone(),
            usage: self.usage.clone(),
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
            auth_session: self.auth_session.clone(),
            parent_tx: self.parent_tx.clone(),
            parent_task_id: self.parent_task_id.clone(),
            dynamic_tools: self.dynamic_tools.clone(),
            hook_prompt_state: self.hook_prompt_state.clone(),
            hook_registry: self.hook_registry.clone(),
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

    /// Store execution result in scratchpad store
    pub async fn store_execution_result(&self, result: &ExecutionResult) -> Result<(), AgentError> {
        // Continue with the processed result
        tracing::debug!("Storing execution result for task_id: {}", self.task_id);
        let exec_entry = ExecutionHistoryEntry {
            thread_id: self.thread_id.clone(),
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            execution_result: result.clone(),
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

        // Clone the Arc to keep stores alive during the async operation
        let scratchpad_store = orchestrator.stores.scratchpad_store.clone();
        scratchpad_store.add_entry(&self.thread_id, entry).await?;
        Ok(())
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
