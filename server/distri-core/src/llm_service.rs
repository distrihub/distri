use crate::agent::ExecutorContext;
use crate::llm::{LLMExecutor, LLMResponse};
use crate::AgentError;
use distri_types::{
    AgentEventType, CreateThreadRequest, LlmDefinition, Message, ModelSettings, RunUsage, Tool,
    ToolCallFormat,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Service for executing LLM requests with proper thread and task management
pub struct LlmExecuteService {
    pub orchestrator: Arc<crate::AgentOrchestrator>,
}

impl LlmExecuteService {
    pub fn new(orchestrator: Arc<crate::AgentOrchestrator>) -> Self {
        Self { orchestrator }
    }

    /// Execute an LLM request with proper thread/task creation
    /// This follows the same pattern as orchestrator.execute_stream()
    pub async fn execute(
        &self,
        user_id: String,
        workspace_id: Option<uuid::Uuid>,
        agent_id: String,
        thread_id: Option<String>,
        run_id: Option<String>,
        parent_task_id: Option<String>,
        messages: Vec<Message>,
        tools: Vec<Arc<dyn Tool>>,
        model_settings: ModelSettings,
        headers: Option<HashMap<String, String>>,
        title: Option<String>,
        external_id: Option<String>,
        is_sub_task: bool,
    ) -> Result<LLMExecuteResult, AgentError> {
        // Generate or use provided thread_id
        let thread_id = thread_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Generate task_id (don't use ephemeral IDs)
        let task_id = uuid::Uuid::new_v4().to_string();

        // Create ExecutorContext with tenant context
        let tenant_context = distri_types::TenantContext::new(user_id.clone(), workspace_id);

        let mut context = ExecutorContext::default();
        context.user_id = user_id.clone();
        context.workspace_id = workspace_id.as_ref().map(|w| w.to_string());
        context.agent_id = agent_id.clone();
        context.thread_id = thread_id.clone();
        context.task_id = task_id.clone();
        context.tenant_context = tenant_context.clone();

        if let Some(run_id) = run_id {
            context.run_id = run_id;
        }
        if let Some(parent) = parent_task_id {
            context.parent_task_id = Some(parent);
        }

        context.orchestrator = Some(self.orchestrator.clone());
        let context = Arc::new(context);

        // Only create thread/task for non-sub-tasks
        // Wrap ONCE at service boundary - stores will read from task-local
        if !is_sub_task {
            // Step 1: Ensure thread exists (like orchestrator.execute_stream line 1230-1241)
            self.ensure_thread_exists(
                &thread_id,
                &agent_id,
                title.as_deref(),
                external_id.as_deref(),
            )
            .await?;

            // Step 2: Get or create task (like orchestrator.execute_stream line 1243-1247)
            self.orchestrator
                .stores
                .task_store
                .get_or_create_task(&thread_id, &task_id)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;

            // Step 3: Set parent task if provided
            if let Some(parent) = context.parent_task_id.as_deref() {
                let _ = self
                    .orchestrator
                    .stores
                    .task_store
                    .update_parent_task(&task_id, Some(parent))
                    .await;
            }
        }

        // Step 4: Save user messages through context (before LLM execution)
        for msg in &messages {
            context.save_message(msg).await;
        }

        // Step 5: Create LLM executor
        let llm_def = LlmDefinition {
            name: format!("llm_execute_{}", model_settings.model),
            model_settings: model_settings.clone(),
            tool_format: ToolCallFormat::Provider,
        };

        let llm = LLMExecutor::new(
            llm_def,
            tools,
            context.clone(),
            headers,
            Some("llm_execute".to_string()),
        );

        // Step 6: Execute LLM (messages are already saved, assistant response will be saved by executor)
        let resp = llm.execute(&messages).await?;

        // Step 7: Emit RunFinished event for usage tracking (non-sub-tasks only)
        if !is_sub_task {
            let context_usage = context.get_usage().await;
            let run_usage = RunUsage {
                total_tokens: context_usage.tokens,
                input_tokens: context_usage.input_tokens,
                output_tokens: context_usage.output_tokens,
                estimated_tokens: context_usage.context_size.total_estimated_tokens as u32,
            };

            context
                .emit(AgentEventType::RunFinished {
                    success: true,
                    total_steps: 1,
                    failed_steps: 0,
                    usage: Some(run_usage),
                })
                .await;

            // Step 8: Update thread with last message
            let last_msg = if !resp.content.is_empty() {
                resp.content.chars().take(200).collect::<String>()
            } else if !resp.tool_calls.is_empty() {
                format!("[Tool calls: {}]", resp.tool_calls.len())
            } else {
                String::new()
            };

            if let Err(e) = self
                .orchestrator
                .stores
                .thread_store
                .update_thread_with_message(&thread_id, &last_msg)
                .await
            {
                tracing::warn!("Failed to update thread with message: {}", e);
            }
        }

        Ok(LLMExecuteResult {
            response: resp,
            thread_id,
            task_id,
        })
    }

    /// Ensure thread exists, following orchestrator.ensure_thread_exists_with_store pattern
    async fn ensure_thread_exists(
        &self,
        thread_id: &str,
        agent_id: &str,
        title: Option<&str>,
        external_id: Option<&str>,
    ) -> Result<(), AgentError> {
        let thread_exists = self
            .orchestrator
            .stores
            .thread_store
            .get_thread(thread_id)
            .await
            .ok()
            .flatten()
            .is_some();

        if !thread_exists {
            let create_req = CreateThreadRequest {
                agent_id: agent_id.to_string(),
                title: title.map(|t| t.to_string()),
                thread_id: Some(thread_id.to_string()),
                attributes: None,
                user_id: None, // Will be set by task-local context
                external_id: external_id.map(|e| e.to_string()),
            };

            self.orchestrator
                .stores
                .thread_store
                .create_thread(create_req)
                .await
                .map_err(|e| AgentError::Session(e.to_string()))?;
        }

        Ok(())
    }
}

/// Result from LLM execution
pub struct LLMExecuteResult {
    pub response: LLMResponse,
    pub thread_id: String,
    pub task_id: String,
}
