use distri::agent::{AgentEventType, AgentExecutor, ExecutorContext};

use distri::types::{Message, MessageRole};

use std::io::{self, Write};
use std::sync::Arc;
use tracing::{error, info};

#[derive(Debug)]
struct MessageBuffer {
    content: String,
    is_complete: bool,
}

#[derive(Debug)]
struct PlanState {
    is_active: bool,
    content: String,
}

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

    let mut current_message: Option<MessageBuffer> = None;
    let mut plan_state = PlanState {
        is_active: false,
        content: String::new(),
    };

    while let Some(event) = rx.recv().await {
        match event.event {
            AgentEventType::RunStarted {} => {
                println!("🚀 Starting agent execution...\n");
            }
            AgentEventType::PlanStarted { initial_plan } => {
                if initial_plan {
                    plan_state.is_active = true;
                    plan_state.content.clear();
                    print!("🤔 Planning... ");
                    io::stdout().flush().unwrap();
                }
            }
            AgentEventType::PlanFinished { plan } => {
                if plan_state.is_active {
                    // Clear the "Planning..." line and show the final plan
                    print!("\r");
                    println!("📋 Plan: {}", plan);
                    plan_state.is_active = false;
                }
            }
            AgentEventType::TextMessageStart { role, .. } => {
                // Complete any previous message
                if let Some(msg) = current_message.take() {
                    if msg.is_complete {
                        println!();
                    }
                }

                let role_display = match role {
                    MessageRole::User => "👤 User",
                    MessageRole::Assistant => "🤖 Assistant",
                    MessageRole::System => "⚙️  System",
                };

                current_message = Some(MessageBuffer {
                    content: String::new(),
                    is_complete: false,
                });

                print!("{}: ", role_display);
                io::stdout().flush().unwrap();
            }
            AgentEventType::TextMessageContent { delta, .. } => {
                if let Some(ref mut msg) = current_message {
                    msg.content.push_str(&delta);
                    print!("{}", delta);
                    io::stdout().flush().unwrap();
                }
            }
            AgentEventType::TextMessageEnd { .. } => {
                if let Some(ref mut msg) = current_message {
                    msg.is_complete = true;
                    println!();
                }
            }
            AgentEventType::ToolCallStart { tool_call_name, .. } => {
                // Complete current message if any
                if let Some(msg) = current_message.take() {
                    if msg.is_complete {
                        println!();
                    }
                }
                println!("🔧 Calling tool: {}", tool_call_name);
            }
            AgentEventType::ToolCallArgs { delta, .. } => {
                // Optionally show tool arguments as they stream
                if verbose && !delta.is_empty() {
                    print!("{}", delta);
                    io::stdout().flush().unwrap();
                }
            }
            AgentEventType::ToolCallEnd { .. } => {
                if verbose {
                    println!();
                }
            }
            AgentEventType::ToolCallResult {
                tool_call_id,
                result,
            } => {
                if verbose {
                    println!("✅ Tool result ({}): {}", tool_call_id, result);
                } else {
                    println!("✅ Tool completed");
                }
            }
            AgentEventType::AgentHandover {
                from_agent,
                to_agent,
                reason,
            } => {
                println!("\n🔄 Agent handover: {} → {}", from_agent, to_agent);
                if let Some(reason) = reason {
                    println!("   Reason: {}", reason);
                }
                println!();
            }
            AgentEventType::RunFinished {} => {
                println!("\n✨ Agent execution completed!");
            }
            AgentEventType::RunError { message, code } => {
                println!("\n❌ Error: {}", message);
                if let Some(code) = code {
                    println!("   Code: {}", code);
                }
            }
        }
    }

    // Complete any remaining message
    if let Some(msg) = current_message.take() {
        if msg.is_complete {
            println!();
        }
    }

    handle.await.unwrap();
    Ok(())
}
