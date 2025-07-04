use distri::agent::AgentExecutor;
use distri::memory::TaskStep;
use std::sync::Arc;
use tracing::{error, info};

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentExecutor>,
    task: TaskStep,
) -> anyhow::Result<()> {
    info!("Executing agent run");
    match executor
        .execute(
            agent_name,
            task,
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
