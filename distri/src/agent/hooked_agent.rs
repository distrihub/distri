use crate::{
    agent::{AgentHooks, BaseAgent, StandardAgentImpl},
    delegate_base_agent,
    error::AgentError,
    memory::TaskStep,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;

/// Agent that wraps StandardAgentImpl and provides hooks to it
pub struct HookedAgent {
    base: StandardAgentImpl,
    hooks: Vec<Arc<dyn AgentHooks>>,
}

impl std::fmt::Debug for HookedAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookedAgent")
            .field("base", &self.base)
            .field("hooks", &self.hooks.len())
            .finish()
    }
}

impl HookedAgent {
    /// Create a new agent with the given hooks
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        hooks: Vec<Arc<dyn AgentHooks>>,
    ) -> Self {
        let base = StandardAgentImpl::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );

        Self { base, hooks }
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
            Vec::new(),
        )
    }

    /// Add a hook to the agent
    pub fn with_hook(mut self, hook: Arc<dyn AgentHooks>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Add multiple hooks to the agent
    pub fn with_hooks(mut self, hooks: Vec<Arc<dyn AgentHooks>>) -> Self {
        self.hooks.extend(hooks);
        self
    }

    /// Get all hooks
    pub fn get_hooks(&self) -> &[Arc<dyn AgentHooks>] {
        &self.hooks
    }

    /// Get the base agent implementation
    pub fn get_base(&self) -> &StandardAgentImpl {
        &self.base
    }

    /// Get mutable access to the base agent implementation
    pub fn get_base_mut(&mut self) -> &mut StandardAgentImpl {
        &mut self.base
    }
}

impl Clone for HookedAgent {
    fn clone(&self) -> Self {
        // Since AgentHooks doesn't implement Clone, we need to recreate the hooks
        // This is a limitation - hooks need to be Clone or we need a different approach
        Self {
            base: self.base.clone(),
            hooks: Vec::new(), // For now, we'll lose hooks on clone
        }
    }
}

// Implement BaseAgent by delegating to the base implementation
delegate_base_agent!(HookedAgent, "HookedAgent", base);

// Implement AgentHooks by chaining all hooks
#[async_trait::async_trait]
impl AgentHooks for HookedAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Call hooks on all registered hooks
        for hook in &self.hooks {
            hook.after_task_step(task.clone(), context.clone()).await?;
        }
        Ok(())
    }

    async fn before_llm_step(
        &self,
        messages: &[crate::types::Message],
        params: &Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
        let mut processed_messages = messages.to_vec();
        
        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_messages = hook
                .before_llm_step(&processed_messages, params, context.clone())
                .await?;
        }
        
        Ok(processed_messages)
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        let mut processed_tool_calls = tool_calls.to_vec();
        
        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_tool_calls = hook
                .before_tool_calls(&processed_tool_calls, context.clone())
                .await?;
        }
        
        Ok(processed_tool_calls)
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Call hooks on all registered hooks
        for hook in &self.hooks {
            hook.after_tool_calls(tool_responses, context.clone()).await?;
        }
        Ok(())
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, AgentError> {
        let mut processed_result = step_result;
        
        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_result = hook
                .after_finish(processed_result, context.clone())
                .await?;
        }
        
        Ok(processed_result)
    }
}