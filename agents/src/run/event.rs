use crate::cli::RunWorkflow;
use std::sync::Arc;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

use agents::{
    executor::AgentExecutor, servers::registry::ServerRegistry, tools::get_tools, AgentDefinition,
    SessionStore,
};

pub async fn run(
    agent: &AgentDefinition,
    registry: Arc<ServerRegistry>,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
    mode: &RunWorkflow,
) -> anyhow::Result<()> {
    let server_tools = get_tools(agent.tools.clone(), registry.clone()).await?;
    let executor = AgentExecutor::new(agent.clone(), registry, session_store, server_tools);
    let messages = Vec::new();

    info!("Running agent (Ctrl+C to stop)...");

    let mut count = 0;
    while let RunWorkflow::Event { times, every } = mode {
        if let Some(every) = every {
            info!("Sleeping for {} seconds before next run", every);
            sleep(Duration::from_secs(*every)).await
        } else {
            let times = times.unwrap_or(1);
            info!("times: {times} count: {count}");
            if count > times {
                break;
            }
        }

        // Check for Ctrl+C
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("\nReceived Ctrl+C, shutting down...");
                break;
            }
            _ = async {
                info!("Executing scheduled agent run - iteration: {count}");
                match executor.execute(messages.clone()).await {
                    Ok(response) => {
                        info!("Agent execution completed successfully");
                        info!("Agent response: {}", response);
                    },
                    Err(e) => error!("Error from agent: {}", e),
                }
            } => {}
        }

        count += 1;
    }

    Ok(())
}
