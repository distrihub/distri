use distri::{
    agent::{create_tool_parser_agent_factory_with_format, AgentExecutorBuilder},
    types::{AgentDefinition, ToolCallFormat},
};
use std::sync::Arc;

/// Example demonstrating how to use different tool call formats
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up logging
    tracing_subscriber::fmt::init();

    println!("🔧 Tool Call Format Example");
    println!("==========================\n");

    // Create agent definitions for different formats
    let current_format_agent = AgentDefinition {
        name: "current_format_agent".to_string(),
        description: "Agent using current XML format".to_string(),
        system_prompt: Some("You are a helpful assistant that can search the web.".to_string()),
        mcp_servers: vec![], // Will be populated by the agent
        ..Default::default()
    };

    let function_format_agent = AgentDefinition {
        name: "function_format_agent".to_string(),
        description: "Agent using JavaScript-like function format".to_string(),
        system_prompt: Some("You are a helpful assistant that can search the web.".to_string()),
        mcp_servers: vec![], // Will be populated by the agent
        ..Default::default()
    };

    // Create executor
    let executor = Arc::new(AgentExecutorBuilder::default().build()?);

    // Register agents with different formats
    println!("📝 Registering agents with different tool call formats...");
    
    // Register current format agent
    executor
        .register_agent_definition_with_factory(
            current_format_agent,
            create_tool_parser_agent_factory_with_format(ToolCallFormat::Current),
        )
        .await?;

    // Register function format agent
    executor
        .register_agent_definition_with_factory(
            function_format_agent,
            create_tool_parser_agent_factory_with_format(ToolCallFormat::Function),
        )
        .await?;

    println!("✅ Agents registered successfully!\n");

    // Example queries to test different formats
    let test_queries = vec![
        "Search for information about Rust programming",
        "Find the latest news about AI",
        "What's the weather like today?",
    ];

    for query in test_queries {
        println!("🔍 Testing query: '{}'", query);
        println!("----------------------------------------");

        // Test current format agent
        println!("📋 Current Format Agent:");
        match executor
            .execute("current_format_agent", query, None, None)
            .await
        {
            Ok(result) => println!("✅ Result: {}", result),
            Err(e) => println!("❌ Error: {}", e),
        }

        println!();

        // Test function format agent
        println!("📋 Function Format Agent:");
        match executor
            .execute("function_format_agent", query, None, None)
            .await
        {
            Ok(result) => println!("✅ Result: {}", result),
            Err(e) => println!("❌ Error: {}", e),
        }

        println!("\n" + "=".repeat(50) + "\n");
    }

    println!("🎉 Tool call format example completed!");
    Ok(())
}