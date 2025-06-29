use std::sync::Arc;

use distri::store::AgentStore;

pub async fn _visualize_agents(agent_store: Arc<dyn AgentStore>) -> anyhow::Result<()> {
    let (agents, _) = agent_store.list(None, None).await;

    // Create a vector to hold the lines for the ASCII graph
    let mut lines = Vec::new();

    for agent in agents.iter() {
        // Add agent representation
        let definition = &agent.definition;
        lines.push("┌─────────────────────────────┐".to_string());
        lines.push(format!("│         {}            │", definition.name));
        lines.push(format!("│  Description: {} │", definition.description));
        lines.push("└─────────────────────────────┘".to_string());

        let tools = agent_store
            .get_tools(&agent.definition.name)
            .await
            .unwrap_or_default();
        // Connect agent to its tools
        for tool in tools {
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
