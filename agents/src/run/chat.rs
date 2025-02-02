use agents::coordinator::{AgentCoordinator, LocalCoordinator};
use agents::store::AgentSessionStore;
use rustyline::DefaultEditor;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use agents::{
    servers::registry::ServerRegistry,
    types::{Message, Role},
    ToolSessionStore,
};

use crate::AgentConfig;

pub async fn run(
    agent_config: &AgentConfig,
    registry: Arc<RwLock<ServerRegistry>>,
    agent_sessions: Option<Arc<Box<dyn AgentSessionStore>>>,
    tool_sessions: Option<Arc<Box<dyn ToolSessionStore>>>,
) -> anyhow::Result<()> {
    let agent = &agent_config.definition;
    let max_history = agent_config.max_history;
    let agent_name = &agent.name;
    let coordinator = LocalCoordinator::new(registry, agent_sessions, tool_sessions);

    coordinator.register_agent(agent.clone()).await?;
    // Set up messages file in .distri folder
    let messages_file = {
        let path = PathBuf::from(".distri");
        std::fs::create_dir_all(&path).unwrap_or_default();
        path.join(format!("{}.messages", agent.name))
    };

    // Load last n messages from file
    let mut messages = if messages_file.exists() {
        let file = File::open(&messages_file)?;
        let reader = BufReader::new(file);
        let all_messages: Vec<Message> = reader
            .lines()
            .filter_map(|line| line.ok().and_then(|l| serde_json::from_str(&l).ok()))
            .collect();
        all_messages
            .into_iter()
            .rev()
            .take(max_history)
            .rev()
            .collect()
    } else {
        Vec::new()
    };

    // Open messages file in append mode for writing
    let mut messages_writer = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&messages_file)?;

    // Create readline editor with history
    let mut rl = DefaultEditor::new()?;

    // Set up history file in .distri folder in current directory
    let history_file = {
        let path = PathBuf::from(".distri");
        std::fs::create_dir_all(&path).unwrap_or_default();
        path.join("history")
    };

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

        // Create user message
        let user_message = Message {
            message: input.to_string(),
            role: Role::User,
            name: None,
        };

        // Append user message to file
        if let Ok(serialized) = serde_json::to_string(&user_message) {
            let _ = writeln!(messages_writer, "{}", serialized);
            let _ = messages_writer.flush();
        }

        // Add message to history
        messages.push(user_message);

        // Execute and print response - only send last n messages to executor
        let context = messages
            .iter()
            .rev()
            .take(max_history)
            .rev()
            .cloned()
            .collect();
        match coordinator.execute(agent_name, context, None).await {
            Ok(response) => {
                println!("{}", response);
                let assistant_message = Message {
                    name: Some(agent.name.clone()),
                    role: Role::Assistant,
                    message: response,
                };

                // Append assistant message to file
                if let Ok(serialized) = serde_json::to_string(&assistant_message) {
                    let _ = writeln!(messages_writer, "{}", serialized);
                    let _ = messages_writer.flush();
                }

                // Add message to history
                messages.push(assistant_message);
            }
            Err(e) => eprintln!("Error from agent: {}", e),
        }
    }

    // Save history one final time before exiting
    let _ = rl.save_history(&history_file);
    Ok(())
}
