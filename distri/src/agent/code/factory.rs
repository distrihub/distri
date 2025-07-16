use crate::agent::code::CodeAgent;
use crate::agent::types::{AgentDefinition, BaseAgent};
use crate::agent::AgentFactoryRegistry;
use crate::stores::SessionStore;
use crate::tools::LlmToolsRegistry;
use std::sync::Arc;

/// Create a factory function for CodeAgent
pub fn create_code_agent_factory() -> Arc<dyn Fn(AgentDefinition, Arc<LlmToolsRegistry>, Arc<crate::agent::executor::AgentExecutor>, Arc<Box<dyn SessionStore>>) -> Box<dyn BaseAgent> + Send + Sync> {
    Arc::new(|definition, tools_registry, executor, session_store| {
        let context = Arc::new(crate::agent::types::ExecutorContext::default());
        let code_agent = CodeAgent::new(definition, tools_registry, executor, session_store, context);
        Box::new(code_agent) as Box<dyn BaseAgent>
    })
}

/// Create a factory function for CodeAgent with specific reasoning mode
pub fn create_code_agent_factory_with_mode(
    reasoning_mode: crate::agent::code::agent::ReasoningMode,
) -> Arc<dyn Fn(AgentDefinition, Arc<LlmToolsRegistry>, Arc<crate::agent::executor::AgentExecutor>, Arc<Box<dyn SessionStore>>) -> Box<dyn BaseAgent> + Send + Sync> {
    Arc::new(move |definition, tools_registry, executor, session_store| {
        let context = Arc::new(crate::agent::types::ExecutorContext::default());
        let code_agent = CodeAgent::new(definition, tools_registry, executor, session_store, context)
            .with_reasoning_mode(reasoning_mode.clone());
        Box::new(code_agent) as Box<dyn BaseAgent>
    })
}

/// Register code agent factories with the registry
pub fn register_code_agent_factories(registry: &mut AgentFactoryRegistry) {
    registry.register_factory("code_agent".to_string(), create_code_agent_factory());
    registry.register_factory("code_agent_hybrid".to_string(), create_code_agent_factory_with_mode(crate::agent::code::agent::ReasoningMode::Hybrid));
    registry.register_factory("code_agent_code_only".to_string(), create_code_agent_factory_with_mode(crate::agent::code::agent::ReasoningMode::CodeOnly));
    registry.register_factory("code_agent_llm_only".to_string(), create_code_agent_factory_with_mode(crate::agent::code::agent::ReasoningMode::LLMOnly));
}