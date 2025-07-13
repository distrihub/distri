use crate::{
    agent::{AgentExecutor, BaseAgent, ExecutorContext, LoggingAgent, FilteringAgent, StandardAgent},
    stores::{AgentFactory, SessionStore},
    tools::LlmToolsRegistry,
    types::AgentDefinition,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Factory for creating LoggingAgent instances
pub struct LoggingAgentFactory;

#[async_trait]
impl AgentFactory for LoggingAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a minimal tools registry for the agent
        let tools_registry = Arc::new(LlmToolsRegistry::new(std::collections::HashMap::new()));
        
        let agent = LoggingAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        );
        
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "LoggingAgent"
    }
}

/// Factory for creating FilteringAgent instances
pub struct FilteringAgentFactory {
    pub banned_words: Vec<String>,
}

impl FilteringAgentFactory {
    pub fn new(banned_words: Vec<String>) -> Self {
        Self { banned_words }
    }
    
    pub fn with_default_banned_words() -> Self {
        Self {
            banned_words: vec![
                "badword".to_string(),
                "inappropriate".to_string(),
                "spam".to_string(),
            ],
        }
    }
}

#[async_trait]
impl AgentFactory for FilteringAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a minimal tools registry for the agent
        let tools_registry = Arc::new(LlmToolsRegistry::new(std::collections::HashMap::new()));
        
        let agent = FilteringAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
            self.banned_words.clone(),
        );
        
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "FilteringAgent"
    }
}

/// Factory for creating StandardAgent instances
pub struct StandardAgentFactory;

#[async_trait]
impl AgentFactory for StandardAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a minimal tools registry for the agent
        let tools_registry = Arc::new(LlmToolsRegistry::new(std::collections::HashMap::new()));
        
        let agent = StandardAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        );
        
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "standard"
    }
}

/// Registry for managing agent factories
pub struct AgentFactoryRegistry {
    factories: std::collections::HashMap<String, Box<dyn AgentFactory>>,
}

impl AgentFactoryRegistry {
    pub fn new() -> Self {
        Self {
            factories: std::collections::HashMap::new(),
        }
    }

    pub fn register_factory(&mut self, factory: Box<dyn AgentFactory>) {
        self.factories.insert(factory.agent_type().to_string(), factory);
    }

    pub fn get_factory(&self, agent_type: &str) -> Option<&Box<dyn AgentFactory>> {
        self.factories.get(agent_type)
    }

    pub fn register_default_factories(&mut self) {
        self.register_factory(Box::new(StandardAgentFactory));
        self.register_factory(Box::new(LoggingAgentFactory));
        self.register_factory(Box::new(FilteringAgentFactory::with_default_banned_words()));
    }
}

impl Default for AgentFactoryRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register_default_factories();
        registry
    }
}