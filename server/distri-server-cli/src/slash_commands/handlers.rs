use std::{collections::HashMap, sync::Arc};

use distri_core::{types::ToolCall, AgentOrchestrator};

use crate::{
    run::{
        inquire_menu::InquireMenu,
        list,
        printer::{COLOR_BRIGHT_CYAN, COLOR_BRIGHT_GREEN, COLOR_RED, COLOR_RESET},
    },
    slash_commands::{executor::SlashCommandExecutor, types::SlashCommandResult},
};

/// Generate help message
pub fn generate_help_message() -> String {
    r#"SLASH COMMANDS:
  /agents          - List and switch agents
  /models          - Show model selection menu
  /available-tools - List all available tools
  /clear           - Clear the current session context
  /help            - Show this help message
  /exit            - Exit the chat

USAGE TIPS:
• Just type naturally - distri will determine the best approach
• Use /agents to switch between available agents
• Tab to autocomplete commands and history
"#
    .to_string()
}

/// Handle special function calls that need conversion to appropriate results
pub fn handle_function_call(handler: &str, args: &[String]) -> SlashCommandResult {
    match handler {
        "set_model" => {
            if let Some(model) = args.first() {
                SlashCommandResult::SetModel {
                    model: model.clone(),
                }
            } else {
                SlashCommandResult::Message("Please specify a model name".to_string())
            }
        }
        _ => SlashCommandResult::Message(format!("Unknown function: {}", handler)),
    }
}

pub async fn handle_slash_command(
    input: &str,
    user_id: &str,
    executor: &Arc<AgentOrchestrator>,
    current_agent: &mut String,
    current_model: &mut String,
) -> SlashCommandResult {
    // Create the slash command executor
    let mut command_executor =
        match SlashCommandExecutor::with_tool_auth_store(executor.stores.tool_auth_store.clone()) {
            Ok(executor) => executor,
            Err(e) => {
                println!(
                    "{}❌ Error initializing command system: {}{}",
                    COLOR_RED, e, COLOR_RESET
                );
                return SlashCommandResult::Continue;
            }
        };

    // Execute the command
    match command_executor.execute(input).await {
        Ok(result) => match result {
            SlashCommandResult::ShowMenu(menu) => {
                // Show the inquire menu
                let mut inquire_menu = InquireMenu::new(menu, executor.clone()).await;
                match inquire_menu.show().await {
                    Ok(menu_result) => {
                        let processed_result =
                            handle_menu_result(menu_result, current_agent, current_model).await;
                        // If the menu result is a ToolCall, we need to process it here
                        match processed_result {
                            SlashCommandResult::ToolCall { tool, parameters } => {
                                SlashCommandResult::ToolCall { tool, parameters }
                            }
                            other => other,
                        }
                    }
                    Err(e) => {
                        println!("{}❌ Menu error: {}{}", COLOR_RED, e, COLOR_RESET);
                        SlashCommandResult::Continue
                    }
                }
            }
            SlashCommandResult::AgentCall { agent, message } => {
                *current_agent = agent.clone();
                println!(
                    "{}✅ Switched to agent: {}{}",
                    COLOR_BRIGHT_GREEN, agent, COLOR_RESET
                );
                if !message.is_empty() && !message.starts_with("Switched to") {
                    SlashCommandResult::Message(message)
                } else {
                    SlashCommandResult::Continue
                }
            }
            SlashCommandResult::ToolCall { tool, parameters } => {
                // Handle generic tool calls via orchestrator
                println!(
                    "{}🔧 Calling tool: {} with parameters: {:?}{}",
                    COLOR_BRIGHT_CYAN, tool, parameters, COLOR_RESET
                );

                match call_tool_via_orchestrator(executor, user_id, &tool, &parameters).await {
                    Ok(result) => {
                        println!(
                            "{}✅ Tool result: {}{}",
                            COLOR_BRIGHT_GREEN, result, COLOR_RESET
                        );
                        SlashCommandResult::Continue
                    }
                    Err(e) => {
                        println!(
                            "{}❌ Tool execution failed: {}{}",
                            COLOR_RED, e, COLOR_RESET
                        );
                        SlashCommandResult::Continue
                    }
                }
            }
            SlashCommandResult::ListTools => {
                // Execute list_tools function
                match list::list_tools(executor.clone(), None).await {
                    Ok(_) => SlashCommandResult::Continue,
                    Err(e) => {
                        println!("{}❌ Error listing tools: {}{}", COLOR_RED, e, COLOR_RESET);
                        SlashCommandResult::Continue
                    }
                }
            }
            SlashCommandResult::SetModel { model } => {
                *current_model = model.clone();
                println!(
                    "{}✅ Model set to: {}{}",
                    COLOR_BRIGHT_GREEN, model, COLOR_RESET
                );
                SlashCommandResult::Continue
            }
            other => other,
        },
        Err(e) => {
            println!("{}❌ Command error: {}{}", COLOR_RED, e, COLOR_RESET);
            SlashCommandResult::Continue
        }
    }
}

async fn handle_menu_result(
    result: SlashCommandResult,
    current_agent: &mut String,
    current_model: &mut String,
) -> SlashCommandResult {
    match result {
        SlashCommandResult::AgentCall { agent, message } => {
            *current_agent = agent.clone();
            println!(
                "{}✅ Switched to agent: {}{}",
                COLOR_BRIGHT_GREEN, agent, COLOR_RESET
            );
            if !message.is_empty() && !message.starts_with("Switched to") {
                SlashCommandResult::Message(message)
            } else {
                SlashCommandResult::Continue
            }
        }
        SlashCommandResult::SetModel { model } => {
            *current_model = model.clone();
            println!(
                "{}✅ Model set to: {}{}",
                COLOR_BRIGHT_GREEN, model, COLOR_RESET
            );
            SlashCommandResult::Continue
        }
        other => other,
    }
}

async fn call_tool_via_orchestrator(
    executor: &Arc<AgentOrchestrator>,
    user_id: &str,
    tool_name: &str,
    parameters: &HashMap<String, String>,
) -> anyhow::Result<String> {
    use distri_core::types::OrchestratorTrait;

    // Convert parameters to serde_json::Value
    let json_params: serde_json::Value = parameters
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    // Create tool call
    let tool_call = ToolCall {
        tool_name: tool_name.to_string(),
        input: json_params,
        tool_call_id: "toolcall_session".to_string(),
    };

    // Use the orchestrator's trait method with no package filter (gets all tools)
    let result = executor
        .call_tool("toolcall_session", user_id, &tool_call)
        .await
        .map_err(|e| anyhow::anyhow!("Tool execution failed: {}", e))?;

    // Convert result to string for display
    Ok(serde_json::to_string_pretty(&result)?)
}
