use distri::agent::{AgentEvent, AgentEventType, AgentExecutor, ExecutorContext};
use distri::types::Message;
use std::sync::Arc;
use tracing::{error, info};

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentExecutor>,
    task: Message,
    verbose: bool,
) -> anyhow::Result<()> {
    info!("Executing agent run");

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);

    let context = Arc::new(ExecutorContext {
        verbose,
        ..Default::default()
    });
    let agent_name = agent_name.to_string();
    let handle = tokio::spawn(async move {
        let res = executor
            .execute_stream(&agent_name, task, context, tx)
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
            AgentEvent {
                event: AgentEventType::ToolCallResult { tool_call_id, .. },
                ..
            } => {
                println!("Tool call result: (tool_call_id: {})", tool_call_id);
            }
            x => {
                println!("{x:?}");
            }
        }
    }
    handle.await.unwrap();

    Ok(())
}
