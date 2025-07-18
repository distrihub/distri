use crate::{
    agent::{AgentEvent, AgentHooks, StandardAgent},
    delegate_base_agent,
    error::AgentError,
    tools::Tool,
    types::{AgentDefinition, Message},
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
        tools: Vec<Arc<dyn Tool>>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        session_store: Arc<Box<dyn SessionStore>>,
        hooks: Vec<Arc<dyn AgentHooks>>,
    ) -> Self {
        let base = StandardAgent::new(definition, tools, coordinator, session_store);

        Self { base, hooks }
    }

    /// Create a standard agent (no additional hooks)
    pub fn standard(
        definition: AgentDefinition,
        tools: Vec<Arc<dyn Tool>>,
        coordinator: Arc<crate::agent::AgentExecutor>,

        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self::new(definition, tools, coordinator, session_store, Vec::new())
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
    async fn before_invoke(
        &self,
        message: Message,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Result<(), AgentError> {
        // Call hooks on all registered hooks
        for hook in &self.hooks {
            hook.before_invoke(message.clone(), context.clone(), event_tx.clone())
                .await?;
        }
        Ok(())
    }

    async fn llm_messages(&self, messages: &[Message]) -> Result<Vec<Message>, AgentError> {
        let mut processed_messages = messages.to_vec();

        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_messages = hook.llm_messages(&processed_messages).await?;
        }

        Ok(processed_messages)
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        let mut processed_tool_calls = tool_calls.to_vec();

        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_tool_calls = hook.before_tool_calls(&processed_tool_calls).await?;
        }

        Ok(processed_tool_calls)
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[Message],
    ) -> Result<Vec<Message>, AgentError> {
        // Call hooks on all registered hooks
        let mut processed_tool_responses = tool_responses.to_vec();
        for hook in &self.hooks {
            processed_tool_responses = hook.after_tool_calls(&processed_tool_responses).await?;
        }
        Ok(processed_tool_responses)
    }

    async fn before_step_result(
        &self,
        step_result: crate::agent::StepResult,
    ) -> Result<crate::agent::StepResult, AgentError> {
        let mut processed_result = step_result;

        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_result = hook.before_step_result(processed_result).await?;
        }

        Ok(processed_result)
    }

    async fn after_execute(
        &self,
        response: crate::llm::LLMResponse,
    ) -> Result<crate::llm::LLMResponse, AgentError> {
        let mut processed_response = response;

        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_response = hook.after_execute(processed_response).await?;
        }

        Ok(processed_response)
    }

    async fn after_execute_stream(
        &self,
        response: crate::llm::StreamResult,
    ) -> Result<crate::llm::StreamResult, AgentError> {
        let mut processed_response = response;

        // Chain hooks through all registered hooks
        for hook in &self.hooks {
            processed_response = hook.after_execute_stream(processed_response).await?;
        }

        Ok(processed_response)
    }
}
