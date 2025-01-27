use rustyline::DefaultEditor;
use std::path::PathBuf;
use std::sync::Arc;

use agents::{
    executor::AgentExecutor, servers::registry::ServerRegistry, tools::get_tools,
    types::UserMessage, AgentDefinition, SessionStore,
};

pub async fn chat(
    agent: &AgentDefinition,
    registry: Arc<ServerRegistry>,
    session_store: Option<Arc<Box<dyn SessionStore>>>,
) -> anyhow::Result<()> {
    let server_tools = get_tools(agent.tools.clone(), registry.clone()).await?;
    let executor = AgentExecutor::new(agent.clone(), registry, session_store, server_tools);
    let mut messages = Vec::new();

    // Create readline editor with history
    let mut rl = DefaultEditor::new()?;

    // Set up history file in user's home directory
    let history_file = dirs::home_dir()
        .map(|mut path| {
            path.push(".distri_history");
            path
        })
        .unwrap_or_else(|| PathBuf::from(".distri_history"));

    // Load history from file
    if history_file.exists() {
        let _ = rl.load_history(&history_file);
    }

    loop {
        // Show prompt and get user input with history support
        let input = match rl.readline("distri> ") {
            Ok(line) => {
                rl.add_history_entry(&line)?;
                // Save history after each command
                let _ = rl.save_history(&history_file);
                line
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("\nExiting...");
                break;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("\nExiting...");
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

        // Add user message to history
        messages.push(UserMessage {
            message: input.to_string(),
            name: None,
        });

        // Execute and print response
        match executor.execute(messages.clone()).await {
            Ok(response) => println!("{}", response),
            Err(e) => eprintln!("Error from agent: {}", e),
        }
    }

    // Save history one final time before exiting
    let _ = rl.save_history(&history_file);
    Ok(())
}
