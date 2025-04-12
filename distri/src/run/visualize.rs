use std::sync::Arc;

use distri::coordinator::{AgentCoordinator, LocalCoordinator};

pub async fn _visualize_agents(coordinator: Arc<LocalCoordinator>) -> anyhow::Result<()> {
    let (agents, _) = coordinator.list_agents(None).await?;

    // Create a vector to hold the lines for the ASCII graph
    let mut lines = Vec::new();

    for (agent, server_tools) in agents
        .iter()
        .zip(coordinator.agent_tools.read().await.values())
    {
        // Add agent representation
        lines.push("┌─────────────────────────────┐".to_string());
        lines.push(format!("│         {}            │", agent.name));
        lines.push(format!("│  Description: {} │", agent.description));
        lines.push("└─────────────────────────────┘".to_string());

        // Connect agent to its tools
        for tool in server_tools {
            for tool in &tool.tools {
                lines.push("         |".to_string());
                lines.push("         |".to_string());
                lines.push("┌───────────────┐".to_string());
                lines.push(format!("│ {}        │", tool.name));
                lines.push("│ Description:  │".to_string());
                lines.push(format!(
                    "│ {}   │",
                    tool.description.clone().unwrap_or_default()
                ));
                lines.push("└───────────────┘".to_string());
            }
        }
        lines.push(String::new());
    }

    // Print the constructed lines
    for line in lines {
        println!("{}", line);
    }

    Ok(())
}
