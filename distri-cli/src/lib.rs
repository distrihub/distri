mod run;

use anyhow::Result;
use clap::{Parser, Subcommand};
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
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

/// CLI commands for Distri
#[derive(Parser)]
#[command(name = "distri")]
#[command(about = "A distributed agent execution platform")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run an agent with a given task
    Run {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
        /// Agent name to run
        #[arg(short, long)]
        agent: String,
        /// Task to execute
        #[arg(short, long)]
        task: String,
        /// Run as server instead of CLI
        #[arg(long)]
        server: bool,
        /// Server host (when running as server)
        #[arg(long, default_value = "localhost")]
        host: String,
        /// Server port (when running as server)
        #[arg(long, default_value = "8000")]
        port: u16,
    },
    /// List available agents
    List {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
    },
    /// Start the server
    Serve {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
        /// Server host
        #[arg(long, default_value = "localhost")]
        host: String,
        /// Server port
        #[arg(long, default_value = "8000")]
        port: u16,
    },
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

/// Initialize KG memory (if needed for specific agents)
pub async fn init_kg_memory(agent: &str) -> Result<Arc<Mutex<FileMemory>>> {
    let mut memory_path = std::path::PathBuf::from(".distri");
    memory_path.push(format!("{agent}.memory"));
    let memory = FileMemory::new(memory_path).await?;
    Ok(Arc::new(Mutex::new(memory)))
}