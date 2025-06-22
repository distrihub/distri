use crate::{
    coordinator::{AgentCoordinator, LocalCoordinator},
    types::{AgentDefinition, Configuration},
};
use anyhow::Result;
use std::sync::Arc;

pub struct DistriEngine {
    pub coordinator: Arc<LocalCoordinator>,
    pub config: Arc<Configuration>,
}

impl DistriEngine {
    pub fn new(coordinator: Arc<LocalCoordinator>, config: Arc<Configuration>) -> Self {
        Self {
            coordinator,
            config,
        }
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentDefinition>> {
        let (agents, _) = self.coordinator.list_agents(None).await?;
        Ok(agents)
    }
}
