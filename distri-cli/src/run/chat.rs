use distri::coordinator::{AgentCoordinator, LocalCoordinator};
use distri::memory::TaskStep;
use distri::types::AgentConfig;
use rustyline::DefaultEditor;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use distri::types::{Message, MessageContent, MessageRole};

pub async fn run(
    agent_config: &AgentConfig,
    coordinator: Arc<LocalCoordinator>,
) -> anyhow::Result<()> {
    let agent = &agent_config.definition;
    let agent_name = &agent.name;

    // Create readline editor with history
    let mut rl = DefaultEditor::new()?;

    // Set up history file in .distri folder in current directory
    let history_file = {
        let path = PathBuf::from(".distri").join(agent_name);
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
            tool_calls: Vec::new(),
        };

        info!("{agent_name}: {user_message:?}");
        match coordinator
            .execute(
                agent_name,
                TaskStep {
                    task: user_message.content[0].text.clone().unwrap(),
                    task_images: None,
                },
                None,
                Arc::default(), // No thread context for CLI chat
                None,
            )
            .await
        {
            Ok(response) => {
                println!("{}", response);
            }
            Err(e) => eprintln!("Error from agent: {}", e),
        }
        coordinator
            .context
            .update_run_id(uuid::Uuid::new_v4().to_string())
            .await;
    }

    // Save history one final time before exiting
    let _ = rl.save_history(&history_file);
    Ok(())
}
