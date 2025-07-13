use distri::{
    agent::AgentExecutor,
    types::{
        AgentDefinition, Configuration, FrontendTool, ModelSettings, RegisterFrontendToolRequest,
    },
};
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create a basic configuration
    let config = Configuration {
        agents: vec![],
        sessions: Default::default(),
        mcp_servers: vec![],
        proxy: None,
        server: None,
        stores: None,
    };

    // Initialize the executor
    let executor = AgentExecutor::initialize(&config).await?;
    let executor = Arc::new(executor);

    // Example 1: Register a notification tool
    let notification_tool = FrontendTool {
        name: "show_notification".to_string(),
        description: "Show a notification to the user in the frontend".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "The title of the notification"
                },
                "message": {
                    "type": "string",
                    "description": "The message content"
                },
                "type": {
                    "type": "string",
                    "enum": ["info", "success", "warning", "error"],
                    "description": "The type of notification"
                }
            },
            "required": ["title", "message"]
        }),
        frontend_resolved: true,
        metadata: Some(json!({
            "category": "ui",
            "version": "1.0.0"
        })),
    };

    let register_request = RegisterFrontendToolRequest {
        tool: notification_tool,
        agent_id: None, // Available to all agents
    };

    let response = executor.register_frontend_tool(register_request).await?;
    println!("Registered notification tool: {:?}", response);

    // Example 2: Register a file upload tool
    let file_upload_tool = FrontendTool {
        name: "upload_file".to_string(),
        description: "Upload a file in the frontend".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file_type": {
                    "type": "string",
                    "enum": ["image", "document", "video"],
                    "description": "The type of file to upload"
                },
                "max_size_mb": {
                    "type": "number",
                    "description": "Maximum file size in MB"
                }
            },
            "required": ["file_type"]
        }),
        frontend_resolved: true,
        metadata: Some(json!({
            "category": "file",
            "version": "1.0.0"
        })),
    };

    let register_request = RegisterFrontendToolRequest {
        tool: file_upload_tool,
        agent_id: Some("assistant".to_string()), // Only available to assistant agent
    };

    let response = executor.register_frontend_tool(register_request).await?;
    println!("Registered file upload tool: {:?}", response);

    // Example 3: List all registered frontend tools
    let tools = executor.get_frontend_tools(None).await;
    println!("All registered frontend tools:");
    for tool in tools {
        println!("  - {}: {}", tool.name, tool.description);
    }

    // Example 4: List tools for a specific agent
    let tools = executor.get_frontend_tools(Some("assistant")).await;
    println!("Frontend tools for assistant agent:");
    for tool in tools {
        println!("  - {}: {}", tool.name, tool.description);
    }

    // Example 5: Create an agent that can use these tools
    let agent_definition = AgentDefinition {
        name: "assistant".to_string(),
        description: "An assistant that can use frontend tools".to_string(),
        version: Some("1.0.0".to_string()),
        system_prompt: Some("You are a helpful assistant that can use frontend tools to interact with the user interface.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: None,
        max_iterations: Some(5),
        skills: vec![],
        sub_agents: vec![],
    };

    let agent = executor.register_default_agent(agent_definition).await?;
    println!("Created agent: {}", agent.get_name());

    // Example 6: Execute a frontend tool (simulation)
    let execute_request = distri::types::ExecuteFrontendToolRequest {
        tool_name: "show_notification".to_string(),
        arguments: json!({
            "title": "Hello",
            "message": "This is a test notification",
            "type": "info"
        }),
        agent_id: "assistant".to_string(),
        thread_id: Some("test-thread".to_string()),
        context: Some(json!({
            "user_id": "test-user"
        })),
    };

    let response = executor.execute_frontend_tool(execute_request).await?;
    println!("Tool execution response: {:?}", response);

    println!("Frontend tools example completed successfully!");
    Ok(())
}