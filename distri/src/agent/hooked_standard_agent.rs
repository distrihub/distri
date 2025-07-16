use crate::{
    agent::{AgentHooks, BaseAgent, StandardAgentImpl},
    delegate_base_agent,
    error::AgentError,
    memory::TaskStep,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;

/// StandardAgentImpl wrapper that provides hooks
pub struct HookedStandardAgent {
    inner: StandardAgentImpl,
    hooks: Option<Arc<dyn AgentHooks>>,
}

impl std::fmt::Debug for HookedStandardAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookedStandardAgent")
            .field("inner", &self.inner)
            .field("has_hooks", &self.hooks.is_some())
            .finish()
    }
}

impl HookedStandardAgent {
    /// Create a new agent with the given hooks
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        hooks: Option<Arc<dyn AgentHooks>>,
    ) -> Self {
        let inner = StandardAgentImpl::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );

        Self { inner, hooks }
    }

    /// Create a standard agent (no additional hooks)
    pub fn standard(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            None,
        )
    }

    /// Create a standard agent with hooks
    pub fn with_hooks(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        hooks: Arc<dyn AgentHooks>,
    ) -> Self {
        Self::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            Some(hooks),
        )
    }

    /// Get the inner agent implementation
    pub fn get_inner(&self) -> &StandardAgentImpl {
        &self.inner
    }

    /// Get mutable access to the inner agent implementation
    pub fn get_inner_mut(&mut self) -> &mut StandardAgentImpl {
        &mut self.inner
    }
}

impl Clone for HookedStandardAgent {
    fn clone(&self) -> Self {
        // Since AgentHooks doesn't implement Clone, we need to recreate the hooks
        // This is a limitation - hooks need to be Clone or we need a different approach
        Self {
            inner: self.inner.clone(),
            hooks: None, // For now, we'll lose hooks on clone
        }
    }
}

// Implement BaseAgent by delegating to the inner implementation
delegate_base_agent!(HookedStandardAgent, "HookedStandardAgent", inner);

// Override get_hooks to provide hooks
impl HookedStandardAgent {
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        self.hooks.as_deref()
    }
}

// Implement AgentHooks by delegating to the stored hooks
#[async_trait::async_trait]
impl AgentHooks for HookedStandardAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        if let Some(hooks) = &self.hooks {
            hooks.after_task_step(task, context).await
        } else {
            Ok(())
        }
    }

    async fn before_llm_step(
        &self,
        messages: &[crate::types::Message],
        params: &Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
        if let Some(hooks) = &self.hooks {
            hooks.before_llm_step(messages, params, context).await
        } else {
            Ok(messages.to_vec())
        }
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        if let Some(hooks) = &self.hooks {
            hooks.before_tool_calls(tool_calls, context).await
        } else {
            Ok(tool_calls.to_vec())
        }
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        if let Some(hooks) = &self.hooks {
            hooks.after_tool_calls(tool_responses, context).await
        } else {
            Ok(())
        }
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, AgentError> {
        if let Some(hooks) = &self.hooks {
            hooks.after_finish(step_result, context).await
        } else {
            Ok(step_result)
        }
    }
}