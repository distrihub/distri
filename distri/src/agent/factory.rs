use crate::{
    agent::{BaseAgent, ExecutorContext, ToolParserAgent},
    error::AgentError,
    tools::LlmToolsRegistry,
    types::AgentDefinition,
    SessionStore,
};
use std::sync::Arc;

/// Factory function type for creating agents
pub type AgentFactoryFn = dyn Fn(
        AgentDefinition,
        Arc<LlmToolsRegistry>,
        Arc<crate::agent::AgentExecutor>,
        Arc<ExecutorContext>,
        Arc<Box<dyn SessionStore>>,
    ) -> Box<dyn BaseAgent>
    + Send
    + Sync;

/// Registry for agent factories
pub struct AgentFactoryRegistry {
    factories: std::collections::HashMap<String, Arc<AgentFactoryFn>>,
}

impl Default for AgentFactoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentFactoryRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            factories: std::collections::HashMap::new(),
        };
        registry.register_default_factories();

        registry
    }

    pub fn register_default_factories(&mut self) {
        // Register default StandardAgent factory
        self.register_factory(
            "standard".to_string(),
            Arc::new(
                |definition, tools_registry, executor, context, session_store| {
                    Box::new(crate::agent::StandardAgentImpl::new(
                        definition,
                        tools_registry,
                        executor,
                        context,
                        session_store,
                    ))
                },
            ),
        );

        self.register_factory(
            "tool_parser".to_string(),
            Arc::new(
                |definition, tools_registry, executor, context, session_store| {
                    Box::new(ToolParserAgent::new(
                        definition,
                        tools_registry,
                        executor,
                        context,
                        session_store,
                        crate::tool_formatter::ToolCallFormat::Current,
                    ))
                },
            ),
        );
    }

    /// Register a factory for a specific agent type
    pub fn register_factory(&mut self, agent_type: String, factory: Arc<AgentFactoryFn>) {
        self.factories.insert(agent_type, factory);
    }

    /// Create an agent using the appropriate factory
    pub fn create_agent(
        &self,
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        executor: Arc<crate::agent::AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Result<Box<dyn BaseAgent>, AgentError> {
        // Determine agent type from definition or use default
        let agent_type = definition.agent_type.as_deref().unwrap_or("standard");

        let factory = self.factories.get(agent_type).ok_or_else(|| {
            AgentError::NotFound(format!("Agent factory for type '{}' not found", agent_type))
        })?;

        Ok(factory(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        ))
    }

    /// Check if a factory exists for the given agent type
    pub fn has_factory(&self, agent_type: &str) -> bool {
        self.factories.contains_key(agent_type)
    }

    /// Get all registered agent types
    pub fn get_agent_types(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }
}
