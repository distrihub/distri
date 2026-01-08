use anyhow::Context;
use distri_auth::get_local_user_id;
use distri_core::agent::AgentOrchestrator;
use distri_core::tools::APPROVAL_REQUEST_TOOL_NAME;
use distri_core::types::{Message, MessageRole, Part, ToolCall, ToolResponse};
use distri_types::configuration::AgentConfig;
use inquire::Text;
use serde_json::json;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use crate::run::autocomplete::DistriAutocomplete;
use crate::run::printer::{
    run_stream_with_printer, COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW,
    COLOR_GRAY, COLOR_RESET,
};
use crate::slash_commands::handle_slash_command;
use crate::slash_commands::types::SlashCommandResult;
use crate::tool_renderers::ToolRendererRegistry;
use crossterm::terminal;

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentOrchestrator>,
    verbose: bool,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
) -> anyhow::Result<()> {
    let mut thread_id = uuid::Uuid::new_v4().to_string();
    let mut current_agent = agent_name.to_string();
    let mut current_model = "gpt-4.1-mini".to_string();
    let user_id = &get_local_user_id();

    // Print welcome header with current status
    print_welcome_header(&current_agent, &current_model);

    // Load history
    let mut history = load_history().unwrap_or_default();

    // Create autocomplete with history and executor
    let mut autocomplete = DistriAutocomplete::new(history.clone(), executor.clone());

    // Populate agent commands asynchronously
    let agent_commands = get_agent_commands(executor.clone()).await;
    autocomplete.update_agent_commands(agent_commands);

    // Initialize browser immediately if the selected agent needs it
    ensure_chat_browser_if_needed(executor.clone(), &current_agent)
        .await
        .context("Failed to prepare browser for chat agent")?;

    loop {
        // Print context status and separator before each prompt
        print_context_status();
        print_separator_line();

        // Get input using inquire with enhanced autocomplete
        let input = match Text::new("> ")
            .with_autocomplete(autocomplete.clone())
            .with_placeholder("/help for commands... Ask me anything")
            .prompt()
        {
            Ok(line) => {
                // Print help options below the input
                print_help_options();
                // Add to history
                if !line.trim().is_empty() && !history.contains(&line) {
                    history.push(line.clone());
                    // Keep only last 100 entries
                    if history.len() > 100 {
                        history.remove(0);
                    }
                    let _ = save_history(&history);
                    // Update autocomplete with new history
                    autocomplete.update_history(history.clone());
                }
                line
            }
            Err(inquire::InquireError::OperationCanceled) => {
                println!("\nExiting... running cleanup");
                break;
            }
            Err(inquire::InquireError::OperationInterrupted) => {
                println!("\nExiting... running cleanup");
                break;
            }
            Err(err) => {
                eprintln!("Error reading input: {}", err);
                continue;
            }
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Handle slash commands
        if input.starts_with('/') {
            match handle_slash_command(
                input,
                user_id,
                &executor,
                &mut current_agent,
                &mut current_model,
            )
            .await
            {
                SlashCommandResult::Continue => continue,
                SlashCommandResult::Exit => break,
                SlashCommandResult::Message(msg) => {
                    // Display the message directly to the user
                    println!("{}", msg);
                    continue;
                }
                SlashCommandResult::ClearContext => {
                    // Reset all IDs to start a new conversation context
                    thread_id = uuid::Uuid::new_v4().to_string();
                    println!("Context cleared - new conversation started");
                    continue;
                }
                SlashCommandResult::ShowMenu(_)
                | SlashCommandResult::AgentCall { .. }
                | SlashCommandResult::ToolCall { .. }
                | SlashCommandResult::ListTools
                | SlashCommandResult::SetModel { .. }
                | SlashCommandResult::CreateWorkflow { .. }
                | SlashCommandResult::CreatePlugin
                | SlashCommandResult::AuthCommand { .. } => {
                    // These cases are handled internally by the slash command system
                    continue;
                }
            }
        }

        // Create user message
        let user_message = Message::user(input.to_string(), None);
        info!("{}: {:?}", current_agent, user_message);

        // Run the stream with interactive mode enabled
        match run_stream_with_printer(
            &current_agent,
            executor.clone(),
            user_message,
            verbose,
            Some(thread_id.clone()),
            Some(&current_model),
            None,
            tool_renderers.clone(),
        )
        .await
        {
            Ok(Some(invoke_result)) => {
                if !invoke_result.tool_calls.is_empty() {
                    // Approval required
                    handle_tool_calls(
                        invoke_result.tool_calls,
                        verbose,
                        Some(thread_id.clone()),
                        &current_agent,
                        executor.clone(),
                        tool_renderers.clone(),
                    )
                    .await?;
                }
                // Continue loop after handling tool calls or normal completion
            }

            Ok(None) => {
                // Normal completion
            }
            Err(e) => {
                eprintln!("Error from agent: {}", e);
            }
        }
    }

    // Save history one final time before exiting
    cleanup_chat_session(executor).await;
    Ok(())
}

async fn ensure_chat_browser_if_needed(
    executor: Arc<AgentOrchestrator>,
    agent_name: &str,
) -> anyhow::Result<()> {
    let needs_browser = match executor.get_agent(agent_name).await {
        Some(AgentConfig::StandardAgent(def)) => def.should_use_browser(),
        _ => false,
    };

    if !needs_browser {
        return Ok(());
    }

    Ok(())
}

async fn cleanup_chat_session(executor: Arc<AgentOrchestrator>) {
    info!("Cleaning up agent session...");
    executor.cleanup();
}

async fn handle_tool_calls(
    tool_calls: Vec<ToolCall>,
    verbose: bool,
    thread_id: Option<String>,
    agent_name: &str,
    executor: Arc<AgentOrchestrator>,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
) -> anyhow::Result<()> {
    // Handle external tool calls

    for tool_call in tool_calls {
        if tool_call.tool_name == APPROVAL_REQUEST_TOOL_NAME {
            println!(
                "{}🔧 Calling tool: {}{}",
                COLOR_BRIGHT_MAGENTA, tool_call.tool_name, COLOR_RESET
            );
            // Show immediate approval prompt
            println!(
                "\n{}🔧 Approval Required{}",
                COLOR_BRIGHT_YELLOW, COLOR_RESET
            );
            print!(
                "{}Do you approve this operation? (y/n): {}",
                COLOR_BRIGHT_YELLOW, COLOR_RESET
            );
            io::stdout().flush().unwrap();
            let mut input = String::new();
            if let Err(e) = io::stdin().read_line(&mut input) {
                eprintln!("Error reading input: {}", e);
                return Err(anyhow::anyhow!("Error reading input"));
            }

            let approved = input.trim().to_lowercase() == "y";

            if approved {
                println!(
                    "{}✅ Operation approved by user.{}",
                    COLOR_BRIGHT_GREEN, COLOR_RESET
                );
                send_approval_response(
                    agent_name,
                    executor.clone(),
                    &tool_call,
                    verbose,
                    thread_id.clone(),
                    tool_renderers.clone(),
                )
                .await?;
            }
        } else {
            continue;
        }
    }
    Ok(())
}
async fn send_approval_response(
    agent_name: &str,
    executor: Arc<AgentOrchestrator>,
    tool_call: &ToolCall,
    verbose: bool,
    thread_id: Option<String>,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
) -> anyhow::Result<()> {
    // Create approval response message
    let approval_result = json!({
        "approved": true, // Since user already approved in the printer
        "reason": "Approved by user",
        "tool_calls": tool_call.input
    });

    let approval_message = Message {
        role: MessageRole::Tool,
        name: Some("approval_response".to_string()),
        parts: vec![Part::ToolResult(ToolResponse::direct(
            tool_call.tool_call_id.clone(),
            tool_call.tool_name.clone(),
            approval_result,
        ))],
        ..Default::default()
    };

    // Send the approval response and continue execution
    run_stream_with_printer(
        agent_name,
        executor,
        approval_message,
        verbose,
        thread_id,
        None,
        None,
        tool_renderers,
    )
    .await?;

    Ok(())
}

fn load_history() -> Result<Vec<String>, std::io::Error> {
    let history_file = get_history_file();
    if history_file.exists() {
        let content = std::fs::read_to_string(&history_file)?;
        Ok(content.lines().map(|s| s.to_string()).collect())
    } else {
        Ok(Vec::new())
    }
}

fn save_history(history: &[String]) -> Result<(), std::io::Error> {
    let history_file = get_history_file();
    if let Some(parent) = history_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&history_file, history.join("\n"))
}

fn get_history_file() -> PathBuf {
    PathBuf::from(".distri").join("history.txt")
}

async fn get_agent_commands(_executor: Arc<AgentOrchestrator>) -> Vec<String> {
    // No /agent commands - agent selection is through /agents menu only
    Vec::new()
}

fn print_welcome_header(agent_name: &str, model_name: &str) {
    // Print a clean header with agent and model info only
    println!();
    println!(
        "{}Agent:{} {} {}• Model:{} {}",
        COLOR_GRAY, COLOR_RESET, agent_name, COLOR_GRAY, COLOR_RESET, model_name
    );
}

fn print_context_status() {
    // Print context status line
    let context_remaining = 12; // This would be calculated from actual context usage

    // Get actual terminal width or use default
    let term_width: usize = if let Ok((w, _)) = terminal::size() {
        w as usize
    } else {
        80 // fallback width
    };

    let status_text = format!("Context left until auto-compact: {}%", context_remaining);

    // Right-align the context status
    let padding = term_width.saturating_sub(status_text.len());

    println!();
    println!(
        "{}{}{}{}",
        " ".repeat(padding),
        COLOR_GRAY,
        status_text,
        COLOR_RESET
    );
}

fn print_separator_line() {
    // Get actual terminal width or use default
    let term_width: usize = if let Ok((w, _)) = terminal::size() {
        w as usize
    } else {
        80 // fallback width
    };

    // Print separator line
    println!("{}", "─".repeat(term_width));
}

fn print_help_options() {
    println!(
        "{}[Tab to autocomplete, /help for commands]{}",
        COLOR_GRAY, COLOR_RESET
    );
}
