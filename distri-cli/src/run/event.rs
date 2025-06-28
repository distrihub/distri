use distri::coordinator::{AgentCoordinator, LocalCoordinator};
use distri::memory::TaskStep;
use std::sync::Arc;
use tracing::{error, info};

use distri::AgentDefinition;

pub async fn run(
    agent: &AgentDefinition,
    coordinator: Arc<LocalCoordinator>,
) -> anyhow::Result<()> {
    let agent_name = &agent.name;

    info!("Executing agent run");
    match coordinator
        .execute(
            agent_name,
            TaskStep {
                task: "Run this workflow".to_string(),
                task_images: None,
            },
            None,
            Arc::default(), // No thread context for event execution
        )
        .await
    {
        Ok(response) => {
            info!("Agent execution completed successfully");
            info!("Agent response: {}", response);
        }
        Err(e) => error!("Error from agent: {}", e),
    }

    Ok(())
}
