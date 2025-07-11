use distri::agent::{AgentEvent, AgentEventType, AgentExecutor};
use distri::memory::TaskStep;
use std::sync::Arc;
use tracing::{error, info};

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentExecutor>,
    task: TaskStep,
) -> anyhow::Result<()> {
    info!("Executing agent run");

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);

    let agent_name = agent_name.to_string();
    let handle = tokio::spawn(async move {
        let res = executor
            .execute_stream(
                &agent_name,
                task,
                None,
                tx,
                Arc::default(), // No thread context for event execution
            )
            .await;
        if let Err(e) = res {
            error!("Error from agent: {}", e);
        }
    });

    while let Some(event) = rx.recv().await {
        match event {
            AgentEvent {
                event: AgentEventType::TextMessageStart { role, .. },
                ..
            } => {
                print!("{}: ", role);
            }
            AgentEvent {
                event: AgentEventType::TextMessageContent { delta, .. },
                ..
            } => {
                print!("{}", delta);
            }
            AgentEvent {
                event: AgentEventType::TextMessageEnd { .. },
                ..
            } => {
                println!();
            }
            x => {
                println!("{x:?}");
            }
        }
    }
    handle.await.unwrap();

    Ok(())
}
