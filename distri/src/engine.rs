use crate::{
    agent::AgentExecutor,
    types::{AgentDefinition, Configuration},
};
use anyhow::Result;
use std::sync::Arc;

pub struct DistriEngine {
    pub executor: Arc<AgentExecutor>,
    pub config: Arc<Configuration>,
}

impl DistriEngine {
    pub fn new(executor: Arc<AgentExecutor>, config: Arc<Configuration>) -> Self {
        Self { executor, config }
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentDefinition>> {
        let (agents, _) = self.executor.agent_store.list(None, None).await;
        let agent_definitions = agents
            .into_iter()
            .map(|agent| agent.get_definition())
            .collect();
        Ok(agent_definitions)
    }
}
