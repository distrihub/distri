use distri::agent::AgentExecutor;
use distri::memory::TaskStep;
use std::sync::Arc;
use tracing::{error, info};

use distri::AgentDefinition;

pub async fn run(agent: &AgentDefinition, coordinator: Arc<AgentExecutor>) -> anyhow::Result<()> {
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
            None,
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
