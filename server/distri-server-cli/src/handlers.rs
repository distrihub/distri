use anyhow::Result;
use std::sync::Arc;

use distri_core::{
    agent::{
        debug::{generate_agent_prompt, generate_agent_response},
        format_validation_table, validate_agent_prompt,
    },
    types::{OrchestratorTrait, ToolCall},
    AgentOrchestrator,
};

/// Handle the toolcall command to execute a tool directly
pub async fn handle_toolcall_command(
    executor: Arc<AgentOrchestrator>,
    user_id: &str,
    tool_name: String,
    input: Option<String>,
) -> Result<()> {
    println!("🔧 Calling tool: {}", tool_name);

    // Parse input JSON or use empty object
    let json_params = match input {
        Some(input_str) => serde_json::from_str(&input_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse input JSON: {}", e))?,
        None => serde_json::json!({}),
    };

    println!(
        "📝 Parameters: {}",
        serde_json::to_string_pretty(&json_params)?
    );

    // Create tool call
    let tool_call = ToolCall {
        tool_name: tool_name.clone(),
        input: json_params,
        tool_call_id: "toolcall_cli".to_string(),
    };

    // Execute tool using orchestrator
    match executor
        .call_tool("toolcall_session", user_id, &tool_call)
        .await
    {
        Ok(result) => {
            println!("✅ Tool execution successful:");
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Err(e) => {
            println!("❌ Tool execution failed: {}", e);
            return Err(anyhow::anyhow!("Tool execution failed: {}", e));
        }
    }

    Ok(())
}

/// Handle the validate-prompt command to validate agent prompt templates
pub async fn handle_validate_prompt_command(
    executor: Arc<AgentOrchestrator>,
    agent_name: String,
) -> Result<()> {
    println!("🔍 Validating prompt for agent: {}", agent_name);

    // Get agent config from orchestrator
    let agent_config = match executor.get_agent(&agent_name).await {
        Some(config) => config,
        None => {
            println!("❌ Agent '{}' not found", agent_name);
            return Err(anyhow::anyhow!("Agent '{}' not found", agent_name));
        }
    };

    // Convert agent config to StandardDefinition
    let agent_def = match agent_config {
        distri_types::configuration::AgentConfig::StandardAgent(def) => def,
        distri_types::configuration::AgentConfig::SequentialWorkflowAgent(_)
        | distri_types::configuration::AgentConfig::DagWorkflowAgent(_)
        | distri_types::configuration::AgentConfig::CustomAgent(_) => {
            println!(
                "ℹ️  Agent '{}' is not a standard agent - prompt validation not applicable",
                agent_name
            );
            return Ok(());
        }
    };

    // Validate the agent's prompt
    let issues = validate_agent_prompt(&agent_def);

    // Display results in a formatted table
    let table_output = format_validation_table(&agent_name, &issues);
    println!("{}", table_output);

    Ok(())
}

/// Handle the generate-prompt command to generate and print agent planning prompts
pub async fn handle_generate_prompt_command(
    executor: Arc<AgentOrchestrator>,
    agent_name: Option<String>,
    task: String,
    verbose: bool,
) -> Result<()> {
    // Default to "distri" agent if none specified (same as run command)
    let agent_name = agent_name.unwrap_or_else(|| "distri".to_string());

    // Generate the planning prompt using shared function
    let messages = generate_agent_prompt(executor, &agent_name, &task, verbose).await?;

    // Display the prompt with formatting options

    // Raw output - just the prompt text
    if let Some(system_message) = messages.first() {
        if let Some(prompt_text) = system_message.as_text() {
            println!("{}", prompt_text);
        } else {
            return Err(anyhow::anyhow!("Generated message is not a text message"));
        }
    } else {
        return Err(anyhow::anyhow!("No prompt messages were generated"));
    }

    Ok(())
}

/// Handle the generate-response command to generate and stream LLM response with parsed tool calls
pub async fn handle_generate_response_command(
    executor: Arc<AgentOrchestrator>,
    agent_name: Option<String>,
    task: String,
    raw: bool,
    verbose: bool,
) -> Result<()> {
    // Default to "distri" agent if none specified (same as run command)
    let agent_name = agent_name.unwrap_or_else(|| "distri".to_string());

    // Generate the LLM response using shared function
    let response = generate_agent_response(executor, &agent_name, &task, raw, verbose).await?;

    // Raw output - just the response content
    println!("{}", response.content);
    if !response.tool_calls.is_empty() {
        println!();
        println!("TOOL_CALLS:");
        for tool_call in &response.tool_calls {
            println!("{}", serde_json::to_string_pretty(tool_call)?);
        }
    }

    Ok(())
}
