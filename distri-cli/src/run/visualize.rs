use std::sync::Arc;

use distri::stores::AgentStore;

pub async fn _visualize_agents(agent_store: Arc<dyn AgentStore>) -> anyhow::Result<()> {
    let (agents, _) = agent_store.list(None, None).await;

    // Create a vector to hold the lines for the ASCII graph
    let mut lines = Vec::new();

    for agent in agents.iter() {
        // Add agent representation
        let definition = agent.get_definition();
        lines.push("┌─────────────────────────────┐".to_string());
        lines.push(format!("│         {}            │", definition.name));
        lines.push(format!("│  Description: {} │", definition.description));
        lines.push("└─────────────────────────────┘".to_string());

        let tools = agent.get_tools();
        // Connect agent to its tools
        for tool in tools {
            lines.push("         |".to_string());
            lines.push("         |".to_string());
            lines.push("┌───────────────┐".to_string());
            lines.push(format!("│ {}        │", tool.get_name()));
            lines.push("│ Description:  │".to_string());
            lines.push(format!("│ {}   │", tool.get_description()));
            lines.push("└───────────────┘".to_string());
        }
    }
    lines.push(String::new());

    // Print the constructed lines
    for line in lines {
        println!("{}", line);
    }

    Ok(())
}
