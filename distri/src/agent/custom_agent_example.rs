use crate::{
    agent::{BaseAgent, ExecutorContext, agent::AgentType, StandardAgent, wrapper::CustomAgentWrapper, factory::AgentFactory},
    memory::TaskStep,
    stores::{SessionStore},
    types::AgentDefinition,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Example custom agent that adds a prefix to all responses
#[derive(Debug)]
pub struct PrefixAgent {
    definition: AgentDefinition,
    prefix: String,
}

impl PrefixAgent {
    pub fn new(definition: AgentDefinition, prefix: String) -> Self {
        Self { definition, prefix }
    }
}

#[async_trait]
impl BaseAgent for PrefixAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("prefix".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        vec![] // This agent doesn't use tools
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(Self {
            definition: self.definition.clone(),
            prefix: self.prefix.clone(),
        })
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    async fn invoke(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        // Simple implementation that just adds a prefix to the task
        Ok(format!("{}: {}", self.prefix, task.task))
    }
}

/// Factory for creating LoggingAgent instances using the wrapper pattern
pub struct LoggingAgentFactory {
    prefix: String,
}

impl LoggingAgentFactory {
    pub fn new(prefix: String) -> Self {
        Self { prefix }
    }
}

#[async_trait]
impl AgentFactory for LoggingAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<crate::agent::AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a standard agent
        let tools_registry = Arc::new(crate::tools::LlmToolsRegistry::new(std::collections::HashMap::new()));
        let standard_agent = StandardAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        );
        
        // Create the wrapper
        let wrapper = CustomAgentWrapper::new(standard_agent);
        
        Ok(Box::new(wrapper))
    }

    fn agent_type(&self) -> &str {
        "logging"
    }
}

/// Factory for creating FilteringAgent instances using the wrapper pattern
pub struct FilteringAgentFactory {
    banned_words: Vec<String>,
}

impl FilteringAgentFactory {
    pub fn new(banned_words: Vec<String>) -> Self {
        Self { banned_words }
    }
}

#[async_trait]
impl AgentFactory for FilteringAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        executor: Arc<crate::agent::AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        // Create a standard agent
        let tools_registry = Arc::new(crate::tools::LlmToolsRegistry::new(std::collections::HashMap::new()));
        let standard_agent = StandardAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        );
        
        // Create the wrapper
        let wrapper = CustomAgentWrapper::new(standard_agent);
        
        Ok(Box::new(wrapper))
    }

    fn agent_type(&self) -> &str {
        "filtering"
    }
}

/// Example custom agent that counts characters in responses
#[derive(Debug)]
pub struct CharCountAgent {
    definition: AgentDefinition,
}

impl CharCountAgent {
    pub fn new(definition: AgentDefinition) -> Self {
        Self { definition }
    }
}

#[async_trait]
impl BaseAgent for CharCountAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("char_count".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        vec![] // This agent doesn't use tools
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(Self {
            definition: self.definition.clone(),
        })
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    async fn invoke(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        // Simple implementation that returns character count
        Ok(format!("Task has {} characters", task.task.len()))
    }
}

/// Factory for creating CharCountAgent instances
pub struct CharCountAgentFactory;

#[async_trait]
impl AgentFactory for CharCountAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        _executor: Arc<crate::agent::AgentExecutor>,
        _context: Arc<ExecutorContext>,
        _session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let agent = CharCountAgent::new(definition);
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "char_count"
    }
}