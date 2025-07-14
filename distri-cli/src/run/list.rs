use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, ContentArrangement, Table};
use distri::{agent::AgentExecutor, tools::get_mcp_tools, types::McpServerType, McpDefinition};
use std::{collections::HashMap, sync::Arc};

pub async fn list(executor: Arc<AgentExecutor>) -> anyhow::Result<()> {
    let (agents, _) = executor.agent_store.list(None, None).await;
    let mut table = Table::new()
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();

    table = table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();
    table.add_row(vec!["Agent", "Description", "Servers"]);
    for definition in agents.iter() {
        let agent = executor
            .create_agent_from_definition(definition.clone())
            .await?;

        let tools = agent.get_tools();
        table.add_row(vec![
            definition.name.clone(),
            definition.description.clone(),
            tools
                .iter()
                .map(|t| t.get_name())
                .collect::<Vec<String>>()
                .join(", "),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub async fn list_tools(executor: Arc<AgentExecutor>) -> anyhow::Result<()> {
    let mut table = Table::new()
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .to_owned();

    let mut map = HashMap::new();
    {
        let registry = executor.registry.clone();
        let servers = registry.read().await;
        let servers = servers.servers.keys();
        for name in servers {
            let def = McpDefinition {
                name: name.clone(),
                r#type: McpServerType::Tool,
                filter: None,
            };
            let tools = get_mcp_tools(&[def], registry.clone()).await?;
            map.insert(name.clone(), tools);
        }
    }

    table.add_row(vec!["Server", "Tools"]);
    for (server_name, server_tools) in map.iter() {
        let mut inner_table = Table::new()
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_width(60)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .to_owned();
        inner_table.add_row(vec!["Tool", "Description"]);
        server_tools.iter().for_each(|t| {
            t.tools.iter().for_each(|t| {
                let description = t.description.clone().unwrap_or_default();
                let description = if description.len() > 60 {
                    &description[..60]
                } else {
                    &description
                };
                inner_table.add_row(vec![t.name.clone(), description.to_string()]);
            });
        });
        table.add_row(vec![server_name.clone(), inner_table.to_string()]);
    }
    println!("{table}");
    Ok(())
    //     let mut inner_table = Table::new()
    //         .load_preset(UTF8_FULL)
    //         .apply_modifier(UTF8_ROUND_CORNERS)
    //         .set_width(60)
    //         .set_content_arrangement(ContentArrangement::Dynamic)
    //         .to_owned();
    //     inner_table.add_row(vec!["Server", "Tools", "Description"]);
    //     let future = tokio::spawn(async move {
    //         let definition = McpDefinition {
    //             name: name.clone(),
    //             r#type: McpServerType::Tool,
    //             filter: ToolsFilter::All,
    //         };
    //         let tools = get_tools(&[definition], registry.clone()).await?;
    //         inner_table.add_row(vec![
    //             server.name.clone(),
    //             tools.len().to_string(),
    //             server.description.clone(),
    //         ]);
    //         Ok(inner_table)
    //     });
    //     futures.push(future);
    //     table.add_row(vec![agent.name.clone(), inner_table.to_string()]);
    // }
    // println!("{table}");
}
