use distri::agent::{AgentExecutor, ExecutorContext};
use rustyline::DefaultEditor;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use distri::types::Message;

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentExecutor>,
    verbose: bool,
) -> anyhow::Result<()> {
    // Create readline editor with history
    let mut rl = DefaultEditor::new()?;

    let thread_id = uuid::Uuid::new_v4().to_string();
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
        let user_message = Message::user(input.to_string(), None);

        info!("{agent_name}: {user_message:?}");
        let context = ExecutorContext {
            thread_id: thread_id.clone(),
            run_id: uuid::Uuid::new_v4().to_string(),
            verbose,
            ..Default::default()
        };
        match executor
            .execute(agent_name, user_message, Arc::new(context), None)
            .await
        {
            Ok(response) => {
                println!("{}", response);
            }
            Err(e) => eprintln!("Error from agent: {}", e),
        }
    }

    // Save history one final time before exiting
    let _ = rl.save_history(&history_file);
    Ok(())
}
