use anyhow::Result;
use distri::{
    agent::{AgentExecutorBuilder, AgentFactoryRegistry},
    types::Configuration,
};
use std::sync::Arc;

/// Load configuration from the definition.yaml file
pub fn load_config() -> Result<Configuration> {
    let yaml_content = include_str!("../definition.yaml");
    let config: Configuration = serde_yaml::from_str(yaml_content)?;
    Ok(config)
}

/// Initialize the agent executor with code agent factories
pub async fn init_agent_executor(config: &Configuration) -> Result<Arc<distri::agent::AgentExecutor>> {
    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await?;

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;

    let executor = Arc::new(executor);

    // Register code agent factories
    let mut factory_registry = AgentFactoryRegistry::new();
    distri::agent::code::register_code_agent_factories(&mut factory_registry);
    
    // Register the factory registry with the executor
    executor
        .register_agent_factory("code_agent".to_string(), factory_registry.get_factory("code_agent").unwrap().clone())
        .await;
    executor
        .register_agent_factory("code_agent_hybrid".to_string(), factory_registry.get_factory("code_agent_hybrid").unwrap().clone())
        .await;
    executor
        .register_agent_factory("code_agent_code_only".to_string(), factory_registry.get_factory("code_agent_code_only").unwrap().clone())
        .await;
    executor
        .register_agent_factory("code_agent_llm_only".to_string(), factory_registry.get_factory("code_agent_llm_only").unwrap().clone())
        .await;

    // Register agents from configuration
    for definition in &config.agents {
        executor
            .register_agent_definition(definition.clone())
            .await?;
    }

    Ok(executor)
}