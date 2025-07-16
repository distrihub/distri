use crate::{
    agent::{AgentHooks, StandardAgent},
    delegate_base_agent,
    error::AgentError,
    memory::TaskStep,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;

/// Plugin-based Agent that combines StandardAgentImpl with hooks
#[derive(Clone)]
pub struct Agent {
    base: StandardAgent,
    hooks: Vec<Arc<dyn AgentHooks>>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("base", &self.base)
            .field("hooks", &self.hooks.len())
            .finish()
    }
}

impl Agent {
    /// Create a new agent with the given hooks
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        session_store: Arc<Box<dyn SessionStore>>,
        hooks: Vec<Arc<dyn AgentHooks>>,
    ) -> Self {
        let base = StandardAgent::new(definition, tools_registry, coordinator, session_store);

        Self { base, hooks }
    }

    /// Create a standard agent (no additional hooks)
    pub fn standard(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,

        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self::new(
            definition,
            tools_registry,
            coordinator,
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

    /// Get the base agent implementation
    pub fn get_base(&self) -> &StandardAgent {
        &self.base
    }

    /// Get mutable access to the base agent implementation
    pub fn get_base_mut(&mut self) -> &mut StandardAgent {
        &mut self.base
    }
}

// Implement BaseAgent by delegating to the base implementation
delegate_base_agent!(Agent, "Agent", base);

// Implement AgentHooks by chaining all hooks
#[async_trait::async_trait]
impl AgentHooks for Agent {
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
            hook.after_tool_calls(tool_responses, context.clone())
                .await?;
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
            processed_result = hook.after_finish(processed_result, context.clone()).await?;
        }

        Ok(processed_result)
    }
}
