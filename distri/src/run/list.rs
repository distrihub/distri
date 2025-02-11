use comfy_table::Table;
use distri::coordinator::{AgentCoordinator, LocalCoordinator};
use std::sync::Arc;

pub async fn list(coordinator: Arc<LocalCoordinator>) -> anyhow::Result<()> {
    let (agents, _) = coordinator.list_agents(None).await?;
    let mut table = Table::new();
    table.add_row(vec!["Agent", "Description", "Servers"]);
    for (agent, server_tools) in agents
        .iter()
        .zip(coordinator.agent_tools.read().await.values())
    {
        table.add_row(vec![
            agent.name.clone(),
            agent.description.clone(),
            server_tools
                .iter()
                .map(|t| t.definition.name.clone())
                .collect::<Vec<String>>()
                .join(", "),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn list_tools(coordinator: Arc<LocalCoordinator>) -> anyhow::Result<()> {
    let (agents, _) = coordinator.list_agents(None).await?;
    let mut table = Table::new();
    table.add_row(vec!["Agent", "Tool"]);
    for (agent, server_tools) in agents
        .iter()
        .zip(coordinator.agent_tools.read().await.values())
    {
        let mut inner_table = Table::new();
        inner_table.add_row(vec!["Server", "Tools", "Description"]);
        for server_tool in server_tools {
            for tool in &server_tool.tools {
                inner_table.add_row(vec![
                    server_tool.definition.name.clone(),
                    tool.name.clone(),
                    tool.description.clone().unwrap_or_default(),
                ]);
            }
        }
        table.add_row(vec![agent.name.clone(), inner_table.to_string()]);
    }
    println!("{table}");
    Ok(())
}
