use distri::agent::{AgentEventType, AgentExecutor, ExecutorContext};

use distri::types::{Message, MessageRole};

use serde_json;
use std::io::{self, Write};
use std::sync::Arc;
use tracing::error;

/// Parse and format distri_execute_code tool arguments
fn parse_execute_code(args: &str) -> bool {
    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(args) {
        if let (Some(thought), Some(code)) = (
            json_value.get("thought").and_then(|v| v.as_str()),
            json_value.get("code").and_then(|v| v.as_str()),
        ) {
            // Clear the line and show formatted output
            print!("\r");
            println!("💭 Thought: {}", thought);
            println!("💻 Code:");
            println!("{}", code);
            return true;
        }
    }
    false
}

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
    let mut current_tool_call: Option<String> = None;
    let mut tool_args_buffer = String::new();

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

                    // Default behavior - print the delta
                    print!("{}", delta);
                    io::stdout().flush().unwrap();
                }
            }
            AgentEventType::TextMessageEnd { .. } => {
                if let Some(ref mut msg) = current_message {
                    msg.is_complete = true;

                    // Check if this complete message contains distri_execute_code JSON
                    if msg.content.contains("\"thought\"") && msg.content.contains("\"code\"") {
                        // Clear the line and try to parse
                        print!("\r");
                        if !parse_execute_code(&msg.content) {
                            // If parsing fails, show the original content
                            println!("{}", msg.content);
                        }
                    } else {
                        // Normal message end
                        println!();
                    }
                }
            }
            AgentEventType::ToolCallStart { tool_call_name, .. } => {
                // Complete current message if any
                if let Some(msg) = current_message.take() {
                    if msg.is_complete {
                        println!();
                    }
                }
                current_tool_call = Some(tool_call_name.clone());
                tool_args_buffer.clear();
                println!("🔧 Calling tool: {} \n", tool_call_name);
            }
            AgentEventType::ToolCallArgs { delta, .. } => {
                // Accumulate tool arguments
                tool_args_buffer.push_str(&delta);

                // Show raw delta for non-distri_execute_code tools
                if let Some(ref tool_name) = current_tool_call {
                    if tool_name != "execute_code" && !delta.is_empty() {
                        print!("{}", delta);
                        io::stdout().flush().unwrap();
                    }
                }
            }
            AgentEventType::ToolCallEnd { .. } => {
                // For distri_execute_code, parse and format the complete arguments
                if let Some(ref tool_name) = current_tool_call {
                    if tool_name == "execute_code" {
                        if !parse_execute_code(&tool_args_buffer) {
                            // Fallback if parsing fails
                            println!("🔧 Tool arguments: {}", tool_args_buffer);
                        }
                    } else {
                        // For other tools, just add a newline if verbose
                        if verbose {
                            println!();
                        }
                    }
                }

                // Clear tool call state
                current_tool_call = None;
                tool_args_buffer.clear();
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
