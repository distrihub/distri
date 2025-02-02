use agents::cli::RunWorkflow;
use agents::coordinator::{AgentCoordinator, LocalCoordinator};
use std::sync::Arc;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

use agents::AgentDefinition;

pub async fn run(
    agent: &AgentDefinition,
    coordinator: Arc<LocalCoordinator>,
    mode: &RunWorkflow,
) -> anyhow::Result<()> {
    let agent_name = &agent.name;

    let messages = Vec::new();

    info!("Running agent (Ctrl+C to stop)...");

    let mut count = 0;
    while let RunWorkflow::Event { times, every } = mode {
        if let Some(every) = every {
            info!("Sleeping for {} seconds before next run", every);
            sleep(Duration::from_secs(*every)).await
        } else {
            let times = times.unwrap_or(1);
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
                match coordinator.execute(agent_name, messages.clone(), None).await {
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
