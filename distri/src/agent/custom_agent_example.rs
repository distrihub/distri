use crate::{
    agent::{BaseAgent, ExecutorContext, agent::AgentType},
    memory::TaskStep,
    stores::{AgentFactory, SessionStore},
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

/// Factory for creating PrefixAgent instances
pub struct PrefixAgentFactory {
    prefix: String,
}

impl PrefixAgentFactory {
    pub fn new(prefix: String) -> Self {
        Self { prefix }
    }
}

#[async_trait]
impl AgentFactory for PrefixAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        _executor: Arc<crate::agent::AgentExecutor>,
        _context: Arc<ExecutorContext>,
        _session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let agent = PrefixAgent::new(definition, self.prefix.clone());
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "prefix"
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