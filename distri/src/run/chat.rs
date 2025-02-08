use distri::coordinator::{AgentCoordinator, LocalCoordinator};
use distri::servers::memory::TaskStep;
use distri::types::AgentConfig;
use rustyline::DefaultEditor;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use distri::types::{Message, MessageContent, MessageRole};

pub async fn run(
    agent_config: &AgentConfig,
    coordinator: Arc<LocalCoordinator>,
) -> anyhow::Result<()> {
    let agent = &agent_config.definition;
    let max_history = agent_config.max_history;
    let agent_name = &agent.name;

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
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(input.to_string()),
                image: None,
            }],
            role: MessageRole::User,
            name: None,
        };

        // Append user message to file
        if let Ok(serialized) = serde_json::to_string(&user_message) {
            let _ = writeln!(messages_writer, "{}", serialized);
            let _ = messages_writer.flush();
        }

        // Add message to history
        // messages.push(user_message);

        // Execute and print response - only send last n messages to executor
        // let context = messages
        //     .iter()
        //     .rev()
        //     .take(max_history)
        //     .rev()
        //     .cloned()
        //     .collect();
        info!("{agent_name}: {user_message:?}");
        match coordinator
            .execute(
                agent_name,
                TaskStep {
                    task: user_message.content[0].text.clone().unwrap(),
                    task_images: None,
                },
                None,
            )
            .await
        {
            Ok(response) => {
                println!("{}", response);
                let assistant_message = Message {
                    name: Some(agent.name.clone()),
                    role: MessageRole::Assistant,
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(response),
                        image: None,
                    }],
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
