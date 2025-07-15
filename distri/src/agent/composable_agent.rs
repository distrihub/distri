use crate::{
    agent::{AgentHooks, BaseAgent, StandardAgent},
    agent::capabilities::{
        AgentCapability, ContentFilteringCapability, ContentFilteringHooks, LoggingCapability,
        LoggingHooks, XmlToolParsingCapability, XmlToolParsingHooks,
    },
    error::AgentError,
    memory::TaskStep,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;
use tracing::info;

/// A composable agent that can combine multiple capabilities
#[derive(Clone)]
pub struct ComposableAgent {
    inner: StandardAgent,
    capabilities: Vec<Box<dyn AgentCapability>>,
    hooks: Vec<Box<dyn AgentHooks>>,
}

impl std::fmt::Debug for ComposableAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposableAgent")
            .field("inner", &self.inner)
            .field("capabilities", &self.capabilities.len())
            .field("hooks", &self.hooks.len())
            .finish()
    }
}

impl ComposableAgent {
    /// Create a new composable agent with the given capabilities
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        capabilities: Vec<Box<dyn AgentCapability>>,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );

        // Create hooks for each capability
        let mut hooks: Vec<Box<dyn AgentHooks>> = Vec::new();
        
        for capability in &capabilities {
            match capability.capability_name() {
                "xml_tool_parsing" => {
                    if let Some(xml_cap) = capability.as_any().downcast_ref::<XmlToolParsingCapability>() {
                        hooks.push(Box::new(XmlToolParsingHooks::new(xml_cap.clone())));
                    }
                }
                "enhanced_logging" => {
                    if let Some(log_cap) = capability.as_any().downcast_ref::<LoggingCapability>() {
                        hooks.push(Box::new(LoggingHooks::new(log_cap.clone())));
                    }
                }
                "content_filtering" => {
                    if let Some(filter_cap) = capability.as_any().downcast_ref::<ContentFilteringCapability>() {
                        hooks.push(Box::new(ContentFilteringHooks::new(filter_cap.clone())));
                    }
                }
                _ => {
                    info!("Unknown capability: {}", capability.capability_name());
                }
            }
        }

        Self {
            inner,
            capabilities,
            hooks,
        }
    }

    /// Create a standard agent (no additional capabilities)
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

    /// Create a tool parser agent
    pub fn tool_parser(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        tool_call_format: crate::tool_formatter::ToolCallFormat,
    ) -> Self {
        let xml_capability = Box::new(XmlToolParsingCapability::new(tool_call_format));
        Self::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            vec![xml_capability],
        )
    }

    /// Create a logging agent
    pub fn logging(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        log_level: String,
    ) -> Self {
        let logging_capability = Box::new(LoggingCapability::new(log_level));
        Self::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            vec![logging_capability],
        )
    }

    /// Create a filtering agent
    pub fn filtering(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
        banned_words: Vec<String>,
    ) -> Self {
        let filtering_capability = Box::new(ContentFilteringCapability::new(banned_words));
        Self::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
            vec![filtering_capability],
        )
    }

    /// Get the agent type based on capabilities
    fn get_agent_type(&self) -> String {
        if self.capabilities.is_empty() {
            "standard".to_string()
        } else {
            // Use the first capability's agent type, or combine them
            let mut types: Vec<String> = self
                .capabilities
                .iter()
                .map(|cap| cap.agent_type().to_string())
                .collect();
            types.sort();
            types.dedup();
            types.join("_")
        }
    }

    /// Get all capability names
    pub fn get_capability_names(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .map(|cap| cap.capability_name().to_string())
            .collect()
    }
}

// Custom implementation to override agent_type and get_hooks
#[async_trait::async_trait]
impl BaseAgent for ComposableAgent {
    fn agent_type(&self) -> crate::agent::agent::AgentType {
        crate::agent::agent::AgentType::Custom(self.get_agent_type())
    }

    fn get_definition(&self) -> crate::types::AgentDefinition {
        self.inner.get_definition()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.inner.get_tools()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: tokio::sync::mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), AgentError> {
        self.inner.invoke_stream(task, params, context, event_tx).await
    }
}

/// Composable hooks that chain multiple hook implementations
pub struct ComposableHooks<'a> {
    agent: &'a ComposableAgent,
}

impl<'a> ComposableHooks<'a> {
    pub fn new(agent: &'a ComposableAgent) -> Self {
        Self { agent }
    }
}

#[async_trait::async_trait]
impl<'a> AgentHooks for ComposableHooks<'a> {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Chain all hooks
        for hook in &self.agent.hooks {
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
        // Chain all hooks, passing the modified messages through
        let mut current_messages = messages.to_vec();
        for hook in &self.agent.hooks {
            current_messages = hook
                .before_llm_step(&current_messages, params, context.clone())
                .await?;
        }
        Ok(current_messages)
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        // Chain all hooks
        let mut current_tool_calls = tool_calls.to_vec();
        for hook in &self.agent.hooks {
            current_tool_calls = hook
                .before_tool_calls(&current_tool_calls, context.clone())
                .await?;
        }
        Ok(current_tool_calls)
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Chain all hooks
        for hook in &self.agent.hooks {
            hook.after_tool_calls(tool_responses, context.clone()).await?;
        }
        Ok(())
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, AgentError> {
        // Chain all hooks, passing the modified result through
        let mut current_result = step_result;
        for hook in &self.agent.hooks {
            current_result = hook
                .after_finish(current_result, context.clone())
                .await?;
        }
        Ok(current_result)
    }
}

// Implement AgentHooks for ComposableAgent by delegating to ComposableHooks
#[async_trait::async_trait]
impl AgentHooks for ComposableAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        ComposableHooks::new(self)
            .after_task_step(task, context)
            .await
    }

    async fn before_llm_step(
        &self,
        messages: &[crate::types::Message],
        params: &Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
        ComposableHooks::new(self)
            .before_llm_step(messages, params, context)
            .await
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        ComposableHooks::new(self)
            .before_tool_calls(tool_calls, context)
            .await
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        ComposableHooks::new(self)
            .after_tool_calls(tool_responses, context)
            .await
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, AgentError> {
        ComposableHooks::new(self)
            .after_finish(step_result, context)
            .await
    }
}