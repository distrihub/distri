mod run;

use anyhow::Result;
use distri::{
    agent::{AgentExecutor, ExecutorContext},
    memory::MemoryConfig,
    servers::{
        kg::FileMemory,
        registry::{init_registry_and_coordinator, ServerRegistry},
    },
    store::InMemoryAgentStore,
    types::{Configuration},
};
use dotenv::dotenv;
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use tracing::debug;

/// Load configuration from file path with environment variable substitution
pub fn load_config(config_path: &str) -> Result<Configuration> {
    // Load .env file if it exists
    dotenv().ok();

    // Read the config file
    let config_str = std::fs::read_to_string(config_path)?;

    // Replace environment variables in the config string
    let config_str = replace_env_vars(&config_str);

    // Parse the YAML
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    debug!("config: {config:?}");
    Ok(config)
}

/// Load configuration from string with environment variable substitution
pub fn load_config_from_str(config_str: &str) -> Result<Configuration> {
    dotenv().ok();
    let config_str = replace_env_vars(config_str);
    let config: Configuration = serde_yaml::from_str(&config_str)?;
    debug!("config: {config:?}");
    Ok(config)
}

/// Replace environment variables in config string ({{ENV_VAR}} format)
pub fn replace_env_vars(content: &str) -> String {
    let mut result = content.to_string();

    // Find all patterns matching {{ENV_VAR}}
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").unwrap();

    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let env_var_name = cap.get(1).unwrap().as_str();

        if let Ok(env_value) = env::var(env_var_name) {
            result = result.replace(full_match, &env_value);
        }
    }

    result
}

/// Initialize all infrastructure from configuration (backward compatibility)
pub async fn init_all(
    config: &Configuration,
) -> Result<(Arc<RwLock<ServerRegistry>>, Arc<AgentExecutor>)> {
    let sessions = config.sessions.clone();

    let local_memories = HashMap::new();
    let tool_sessions = run::session::get_session_store(sessions);

    let memory_config = MemoryConfig::File(".distri/memory".to_string());
    let context = Arc::new(ExecutorContext::default());
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let (registry, coordinator) = init_registry_and_coordinator(
        local_memories,
        tool_sessions.clone(),
        agent_store.clone(),
        &config.mcp_servers,
        context,
        memory_config,
    )
    .await;

    Ok((registry, coordinator))
}

/// Initialize AgentExecutor from configuration (recommended)
pub async fn initialize_executor(config: &Configuration) -> Result<Arc<AgentExecutor>> {
    let executor = AgentExecutor::initialize(config).await?;
    Ok(Arc::new(executor))
}

/// Initialize AgentExecutor from config file path (recommended)
pub async fn initialize_executor_from_file(config_path: &str) -> Result<Arc<AgentExecutor>> {
    let config = load_config(config_path)?;
    initialize_executor(&config).await
}

/// Initialize AgentExecutor from config string (recommended)  
pub async fn initialize_executor_from_str(config_str: &str) -> Result<Arc<AgentExecutor>> {
    let config = load_config_from_str(config_str)?;
    initialize_executor(&config).await
}

/// Initialize KG memory (if needed for specific agents)
pub async fn init_kg_memory(agent: &str) -> Result<Arc<Mutex<FileMemory>>> {
    let mut memory_path = std::path::PathBuf::from(".distri");
    memory_path.push(format!("{agent}.memory"));
    let memory = FileMemory::new(memory_path).await?;
    Ok(Arc::new(Mutex::new(memory)))
}