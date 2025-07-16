use code_agent::{init_agent_executor, load_config};
use distri::agent::ExecutorContext;
use distri::types::Message;
use std::sync::Arc;
use tracing::info;

mod lib;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = load_config()?;
    info!("Loaded configuration with {} agents", config.agents.len());

    // Initialize executor
    let executor = init_agent_executor(&config).await?;
    info!("Initialized agent executor");

    // Example tasks to demonstrate different capabilities
    let tasks = vec![
        "Calculate the factorial of 10",
        "Generate a list of prime numbers up to 100",
        "Explain the difference between synchronous and asynchronous programming",
        "Create a function that sorts an array of objects by a specific property",
        "Analyze the time complexity of a binary search algorithm",
    ];

    for (i, task) in tasks.iter().enumerate() {
        println!("\n=== Task {}: {} ===", i + 1, task);
        
        // Test with different agent types
        let agents = vec!["code-agent", "code-agent-hybrid", "code-agent-code-only"];
        
        for agent_name in agents {
            println!("\n--- Using {} ---", agent_name);
            
            let message = Message::user(task.to_string(), None);
            let context = ExecutorContext {
                thread_id: format!("task_{}", i + 1),
                run_id: format!("run_{}_{}", i + 1, agent_name),
                verbose: true,
                user_id: None,
                metadata: None,
                req_id: None,
            };

            match executor.execute(agent_name, message, Arc::new(context), None).await {
                Ok(response) => {
                    println!("Response: {}", response);
                }
                Err(e) => {
                    println!("Error: {}", e);
                }
            }
        }
    }

    Ok(())
}