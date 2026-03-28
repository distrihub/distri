use std::path::PathBuf;

use anyhow::Result;
use distri::Distri;

use crate::{ThreadsCommands, COLOR_GRAY, COLOR_RESET};

pub fn get_last_thread_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".distri").join("last_thread")
}

pub fn save_last_thread(thread_id: &str) -> Result<(), std::io::Error> {
    let path = get_last_thread_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, thread_id)
}

pub fn load_last_thread() -> Option<String> {
    let path = get_last_thread_file();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve a resume argument: "last" → load saved thread ID, otherwise use as-is.
pub fn resolve_resume_arg(arg: &str) -> String {
    if arg == "last" {
        load_last_thread().unwrap_or_else(|| {
            eprintln!("No previous thread saved. Starting fresh.");
            uuid::Uuid::new_v4().to_string()
        })
    } else {
        arg.to_string()
    }
}

/// Fetch and print thread history for a resumed thread.
/// Replays messages and events through `EventPrinter` so the output
/// matches the live streaming format (including tool calls, ctrl+o toggle, etc).
pub async fn print_thread_history(client: &Distri, thread_id: &str) {
    match client.get_thread_messages(thread_id, false).await {
        Ok(items) => {
            if items.is_empty() {
                println!("{}(no previous messages){}", COLOR_GRAY, COLOR_RESET);
                return;
            }
            let msg_count = items
                .iter()
                .filter(|i| matches!(i, distri_types::TaskMessage::Message(_)))
                .count();
            let event_count = items
                .iter()
                .filter(|i| matches!(i, distri_types::TaskMessage::Event(_)))
                .count();
            println!(
                "{}── Thread history ({} messages, {} events) ──{}",
                COLOR_GRAY, msg_count, event_count, COLOR_RESET
            );

            let mut printer = distri::EventPrinter::new();
            for item in &items {
                match item {
                    distri_types::TaskMessage::Event(task_event) => {
                        let agent_event = distri_types::events::AgentEvent::from_task_event(
                            task_event,
                            thread_id,
                        );
                        printer.handle_event(&agent_event).await;
                    }
                    distri_types::TaskMessage::Message(msg) => {
                        // Show user messages as-is, assistant messages as previews
                        let text = msg.parts.iter().find_map(|p| match p {
                            distri_types::Part::Text(t) if !t.is_empty() => Some(t.as_str()),
                            _ => None,
                        });
                        if let Some(text) = text {
                            match msg.role {
                                distri_types::MessageRole::User => {
                                    println!("{}> {}{}", COLOR_GRAY, text, COLOR_RESET);
                                }
                                distri_types::MessageRole::Assistant => {
                                    let preview = if text.len() > 200 {
                                        format!("{}…", &text[..200])
                                    } else {
                                        text.to_string()
                                    };
                                    println!("{}◆ {}{}", COLOR_GRAY, preview, COLOR_RESET);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            println!(
                "{}── End of history ──{}",
                COLOR_GRAY, COLOR_RESET
            );
            println!();
        }
        Err(err) => {
            eprintln!(
                "{}Could not load thread history: {}{}",
                COLOR_GRAY, err, COLOR_RESET
            );
        }
    }
}

pub async fn handle_threads_command(client: &Distri, command: ThreadsCommands) -> Result<()> {
    match command {
        ThreadsCommands::List => {
            let threads = client.list_threads().await?;
            if threads.is_empty() {
                println!("No threads found.");
            } else {
                for thread in threads {
                    let agent = thread.agent_name.as_deref().unwrap_or("unknown");
                    let title = thread.title.as_deref().unwrap_or("(no title)");
                    println!("{} - {} [{}]", thread.id, title, agent);
                }
            }
        }
    }
    Ok(())
}
