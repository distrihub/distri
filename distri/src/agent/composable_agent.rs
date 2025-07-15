use crate::{
    agent::{AgentHooks, BaseAgent, StandardAgent},
    agent::capabilities::{AgentCapability, ContentFilteringCapability, LoggingCapability, XmlToolParsingCapability},
    error::AgentError,
    memory::TaskStep,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;
use tracing::info;

/// A composable agent that can combine multiple capabilities dynamically
pub struct ComposableAgent {
    inner: StandardAgent,
    capabilities: Vec<Box<dyn AgentCapability>>,
}

impl std::fmt::Debug for ComposableAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposableAgent")
            .field("inner", &self.inner)
            .field("capabilities", &self.capabilities.len())
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

        Self {
            inner,
            capabilities,
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

    /// Add a capability to the agent
    pub fn with_capability(mut self, capability: Box<dyn AgentCapability>) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Add multiple capabilities to the agent
    pub fn with_capabilities(mut self, capabilities: Vec<Box<dyn AgentCapability>>) -> Self {
        self.capabilities.extend(capabilities);
        self
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

    /// Get a capability by name
    pub fn get_capability<T: 'static>(&self, name: &str) -> Option<&T> {
        self.capabilities
            .iter()
            .find(|cap| cap.capability_name() == name)
            .and_then(|cap| cap.as_any().downcast_ref::<T>())
    }

    /// Get all capabilities
    pub fn get_capabilities(&self) -> &[Box<dyn AgentCapability>] {
        &self.capabilities
    }
}

impl Clone for ComposableAgent {
    fn clone(&self) -> Self {
        // Since AgentCapability doesn't implement Clone, we need to recreate capabilities
        // This is a limitation of the current design - capabilities need to be Clone
        let mut capabilities = Vec::new();
        
        for capability in &self.capabilities {
            // Try to clone each capability by downcasting and recreating
            if let Some(xml_cap) = capability.as_any().downcast_ref::<XmlToolParsingCapability>() {
                capabilities.push(Box::new(xml_cap.clone()) as Box<dyn AgentCapability>);
            } else if let Some(log_cap) = capability.as_any().downcast_ref::<LoggingCapability>() {
                capabilities.push(Box::new(log_cap.clone()) as Box<dyn AgentCapability>);
            } else if let Some(filter_cap) = capability.as_any().downcast_ref::<ContentFilteringCapability>() {
                capabilities.push(Box::new(filter_cap.clone()) as Box<dyn AgentCapability>);
            } else {
                // For unknown capabilities, we can't clone them
                // This is a limitation that could be addressed by adding Clone to AgentCapability trait
                panic!("Cannot clone unknown capability type");
            }
        }
        
        Self {
            inner: self.inner.clone(),
            capabilities,
        }
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

/// Dynamic hooks implementation that chains all capability hooks
#[async_trait::async_trait]
impl AgentHooks for ComposableAgent {
    async fn after_task_step(
        &self,
        task: TaskStep,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Call hooks on all capabilities
        for capability in &self.capabilities {
            if let Some(hooks) = capability.get_hooks() {
                hooks.after_task_step(task.clone(), context.clone()).await?;
            }
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
        
        // Chain hooks through all capabilities
        for capability in &self.capabilities {
            if let Some(hooks) = capability.get_hooks() {
                processed_messages = hooks
                    .before_llm_step(&processed_messages, params, context.clone())
                    .await?;
            }
        }
        
        Ok(processed_messages)
    }

    async fn before_tool_calls(
        &self,
        tool_calls: &[crate::types::ToolCall],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::ToolCall>, AgentError> {
        let mut processed_tool_calls = tool_calls.to_vec();
        
        // Chain hooks through all capabilities
        for capability in &self.capabilities {
            if let Some(hooks) = capability.get_hooks() {
                processed_tool_calls = hooks
                    .before_tool_calls(&processed_tool_calls, context.clone())
                    .await?;
            }
        }
        
        Ok(processed_tool_calls)
    }

    async fn after_tool_calls(
        &self,
        tool_responses: &[String],
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), AgentError> {
        // Call hooks on all capabilities
        for capability in &self.capabilities {
            if let Some(hooks) = capability.get_hooks() {
                hooks.after_tool_calls(tool_responses, context.clone()).await?;
            }
        }
        Ok(())
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, AgentError> {
        let mut processed_result = step_result;
        
        // Chain hooks through all capabilities
        for capability in &self.capabilities {
            if let Some(hooks) = capability.get_hooks() {
                processed_result = hooks
                    .after_finish(processed_result, context.clone())
                    .await?;
            }
        }
        
        Ok(processed_result)
    }
}