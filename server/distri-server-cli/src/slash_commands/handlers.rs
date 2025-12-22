use std::{collections::HashMap, sync::Arc};

use distri_core::{types::ToolCall, AgentOrchestrator};

use crate::slash_commands::types::{InteractiveMenu, MenuItem, SlashCommandType};
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
    r#"AGENTS AVAILABLE:
• distri       - General-purpose agent with built-in capabilities
• deepresearch - Comprehensive research and analysis

SLASH COMMANDS:
  /agents          - Switch between distri and deepresearch
  /workflows       - Create new or view existing workflows
  /plugins         - Install and manage DAP plugins
  /models          - Show model selection menu
  /available-tools - List all available tools
  /clear           - Clear the current session context
  /help            - Show this help message
  /exit            - Exit the chat

USAGE TIPS:
• Just type naturally - distri will determine the best approach
• Use /agents to switch to deepresearch for complex analysis
• Tab to autocomplete commands and history
"#
    .to_string()
}

/// Create workflows menu (placeholder - actual implementation is in executor)
pub fn create_workflows_menu() -> InteractiveMenu {
    InteractiveMenu {
        title: "Workflows".to_string(),
        items: vec![MenuItem {
            id: "create".to_string(),
            display: "📝 Create new workflow".to_string(),
            description: Some("Generate a new workflow interactively".to_string()),
            action: SlashCommandType::Function {
                handler: "create_workflow_interactive".to_string(),
            },
        }],
        allow_create: false,
        current_selection: 0,
    }
}

/// Create plugins menu (placeholder - actual implementation is in executor)
pub fn create_plugins_menu() -> InteractiveMenu {
    InteractiveMenu {
        title: "Plugins".to_string(),
        items: vec![
            MenuItem {
                id: "install".to_string(),
                display: "📦 Install plugin from registry".to_string(),
                description: Some("Search and install plugins from DAP registry".to_string()),
                action: SlashCommandType::Function {
                    handler: "install_plugin_interactive".to_string(),
                },
            },
            MenuItem {
                id: "create".to_string(),
                display: "🔨 Create new plugin".to_string(),
                description: Some(
                    "Create a new plugin with agents, tools, and workflows".to_string(),
                ),
                action: SlashCommandType::Function {
                    handler: "create_plugin_interactive".to_string(),
                },
            },
        ],
        allow_create: true,
        current_selection: 0,
    }
}

/// Handle special function calls that need conversion to appropriate results
pub fn handle_function_call(handler: &str, args: &[String]) -> SlashCommandResult {
    match handler {
        "create_workflow_interactive" => SlashCommandResult::CreateWorkflow {
            description: args.join(" "),
        },
        "install_plugin_interactive" => SlashCommandResult::Message(
            "Enter plugin name to install from DAP registry (e.g., 'search_agent_demo'):"
                .to_string(),
        ),
        "create_plugin_interactive" => SlashCommandResult::CreatePlugin,
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
                                // Handle the tool call directly here
                                match tool.as_str() {
                                    "workflow_details" => {
                                        let workflow_name = parameters
                                            .get("name")
                                            .cloned()
                                            .unwrap_or_else(|| "unknown".to_string());
                                        let workflow_path = parameters.get("path");

                                        display_workflow_dag(&workflow_name, workflow_path);
                                        SlashCommandResult::Continue
                                    }
                                    _ => {
                                        // Handle other tool calls generically
                                        SlashCommandResult::ToolCall { tool, parameters }
                                    }
                                }
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
                match tool.as_str() {
                    "plugin_details" => {
                        let plugin_name = parameters
                            .get("name")
                            .cloned()
                            .unwrap_or_else(|| "unknown".to_string());

                        println!(
                            "{}🔌 Plugin Details: {}{}",
                            COLOR_BRIGHT_CYAN, plugin_name, COLOR_RESET
                        );

                        // Show plugin location
                        println!("\n{}📂 Location:{}", COLOR_BRIGHT_GREEN, COLOR_RESET);
                        println!("  .distri/plugins/{}", plugin_name);

                        // Show available components (agents, tools, workflows)
                        println!(
                            "\n{}📋 Available Components:{}",
                            COLOR_BRIGHT_GREEN, COLOR_RESET
                        );
                        println!("  🤖 Agents: Check distri.toml for agent definitions");
                        println!("  🔧 Tools: Check distri.toml for tool definitions");
                        println!("  🔗 Workflows: Check distri.toml for workflow definitions");

                        println!(
                            "\n{}💡 Tip: Plugin components are automatically available through the plugin system{}",
                            COLOR_BRIGHT_CYAN, COLOR_RESET
                        );

                        SlashCommandResult::Continue
                    }
                    _ => {
                        // Handle generic tool calls via orchestrator
                        println!(
                            "{}🔧 Calling tool: {} with parameters: {:?}{}",
                            COLOR_BRIGHT_CYAN, tool, parameters, COLOR_RESET
                        );

                        match call_tool_via_orchestrator(executor, user_id, &tool, &parameters)
                            .await
                        {
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
            SlashCommandResult::CreateWorkflow { description } => {
                if description.is_empty() {
                    println!(
                        "{}📝 Starting interactive workflow creation...{}",
                        COLOR_BRIGHT_GREEN, COLOR_RESET
                    );
                    println!(
                        "{}💡 Tip: Describe what you want the workflow to do{}",
                        COLOR_BRIGHT_CYAN, COLOR_RESET
                    );
                    SlashCommandResult::Message(
                        "What should this workflow do? Describe its purpose and functionality."
                            .to_string(),
                    )
                } else {
                    println!(
                        "{}📝 Creating workflow with description: {}{}",
                        COLOR_BRIGHT_GREEN, description, COLOR_RESET
                    );

                    // Switch to workflow-designer agent to create the workflow
                    *current_agent = "workflow-designer".to_string();
                    println!(
                        "{}✅ Switched to workflow-designer agent{}",
                        COLOR_BRIGHT_GREEN, COLOR_RESET
                    );

                    SlashCommandResult::Message(format!(
                        "Create a TypeScript workflow with the following requirements: {}\n\n\
                        Please generate a complete workflow file that can be saved to the .distri folder and executed with the distri CLI.",
                        description
                    ))
                }
            }
            SlashCommandResult::CreatePlugin => {
                println!(
                    "{}🔨 Creating new plugin...{}",
                    COLOR_BRIGHT_GREEN, COLOR_RESET
                );

                println!(
                    "{}📋 Plugin Creation Wizard{}",
                    COLOR_BRIGHT_CYAN, COLOR_RESET
                );
                println!("This will create a new plugin in .distri/plugins/");
                println!();

                // Show available templates
                println!(
                    "{}📝 Available templates:{}",
                    COLOR_BRIGHT_CYAN, COLOR_RESET
                );
                println!(
                    "  • {}typescript{} - TypeScript plugin with agents and tools",
                    COLOR_BRIGHT_GREEN, COLOR_RESET
                );
                println!(
                    "  • {}wasm{} - WASM component plugin",
                    COLOR_BRIGHT_GREEN, COLOR_RESET
                );
                println!(
                    "  • {}full{} - Full-featured plugin with agents, tools, and workflows",
                    COLOR_BRIGHT_GREEN, COLOR_RESET
                );
                println!();

                SlashCommandResult::Message(
                    "Enter plugin name and template (e.g., 'my_plugin typescript'): ".to_string(),
                )
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

/// Display workflow DAG visualization with improved formatting
fn display_workflow_dag(workflow_name: &str, _workflow_path: Option<&String>) {
    println!();
    println!(
        "{}📊 Workflow DAG: {}{}",
        COLOR_BRIGHT_CYAN, workflow_name, COLOR_RESET
    );
    println!();

    // Use enhanced fallback visualization (parsing will be implemented later)

    println!("TODO::");
}
