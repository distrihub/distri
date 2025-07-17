use std::sync::Arc;

use crate::{
    agent::{AgentExecutor, AgentHooks, StandardAgent},
    delegate_base_agent,
    tools::LlmToolsRegistry,
    AgentDefinition, SessionStore,
};

#[derive(Debug, Clone)]

pub struct CodeAgent {
    base: StandardAgent,
}

impl CodeAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<LlmToolsRegistry>,
        coordinator: Arc<AgentExecutor>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        Self {
            base: StandardAgent::new(definition, tools_registry, coordinator, session_store),
        }
    }
}

impl AgentHooks for CodeAgent {}
delegate_base_agent!(CodeAgent, "code", base);
