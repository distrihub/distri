//! Tool simulator for dry-run / eval mode.
//!
//! Wraps tool execution: whitelisted tools execute normally,
//! all others get a simulated response from a cheap LLM (gpt-5-nano).

use crate::types::{Part, ToolCall};
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequest,
};
use async_openai::Client;
use serde_json::json;

/// Tools that are safe to execute for real during dry-run (read-only, no side effects).
const SAFE_TOOLS: &[&str] = &[
    "tool_search",
    "load_skill",
    "get_task",      // read-only supervisor query
    "list_my_tasks", // read-only
    "search",        // web search (read-only)
    "browsr_scrape", // web scrape (read-only)
    "final",
    "write_todos", // todos are session-scoped, safe
];

/// Check if a tool is safe to execute for real in dry-run mode.
pub fn is_safe_tool(tool_name: &str) -> bool {
    SAFE_TOOLS.contains(&tool_name)
}

/// Tools that should always be simulated (have side effects).
#[allow(dead_code)]
const ALWAYS_SIMULATE: &[&str] = &[
    "http_request",  // makes HTTP requests
    "start_shell",   // starts a shell session
    "execute_shell", // runs commands
    "inject_connection_env",
    "create_skill",
    "create_note",
    "update_note",
    "delete_note",
];

/// Simulate a tool call response using a cheap LLM.
///
/// Sends the tool name, parameters, and input to gpt-5-nano and asks it
/// to generate a plausible response matching the tool's expected output.
/// The model the simulator calls (on the platform's Azure deployment).
pub const SIMULATOR_MODEL: &str = "gpt-5-nano";

/// Simulate a tool's response with an LLM (dry-run mode). The call is one
/// LLM call of the run like any other: it is checked against the
/// orchestrator's spend policy and recorded (`purpose = tool_simulation`)
/// through [`crate::llm_call::LlmCallRecorder`].
pub async fn simulate_tool_response(
    context: std::sync::Arc<crate::agent::context::ExecutorContext>,
    tool_call: &ToolCall,
    tool_description: &str,
    tool_parameters: &serde_json::Value,
) -> Result<Vec<Part>, anyhow::Error> {
    let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap_or_default();
    let api_key = std::env::var("AZURE_OPENAI_KEY").unwrap_or_default();

    if endpoint.is_empty() || api_key.is_empty() {
        // Fallback: return a generic simulated response
        return Ok(vec![Part::Data(json!({
            "_simulated": true,
            "status": "ok",
            "message": format!("Simulated response for tool '{}'. Configure AZURE_OPENAI_ENDPOINT and AZURE_OPENAI_KEY for LLM-based simulation.", tool_call.tool_name)
        }))]);
    }

    let config = async_openai::config::OpenAIConfig::new()
        .with_api_base(endpoint)
        .with_api_key(api_key);
    let client = Client::with_config(config);

    let system_text = format!(
        "You are a tool response simulator. Given a tool call, generate a realistic JSON response \
             that the tool would return. Keep it concise and realistic. Mark the response with \
             \"_simulated\": true.\n\n\
             Tool: {}\n\
             Description: {}\n\
             Parameters schema: {}",
        tool_call.tool_name,
        tool_description,
        serde_json::to_string_pretty(tool_parameters).unwrap_or_default()
    );
    let user_text = format!(
        "Simulate the response for this tool call:\n\
         Tool: {}\n\
         Input: {}",
        tool_call.tool_name,
        serde_json::to_string_pretty(&tool_call.input).unwrap_or_default()
    );
    let system_msg = ChatCompletionRequestSystemMessageArgs::default()
        .content(system_text.clone())
        .build()?
        .into();
    let user_msg = ChatCompletionRequestUserMessageArgs::default()
        .content(user_text.clone())
        .build()?
        .into();

    let request = CreateChatCompletionRequest {
        model: SIMULATOR_MODEL.to_string(),
        messages: vec![system_msg, user_msg],
        max_completion_tokens: Some(500),
        temperature: Some(0.3),
        ..Default::default()
    };

    let recorder = match crate::llm_call::LlmCallRecorder::begin(
        context,
        SIMULATOR_MODEL,
        "azure_openai",
        false,
        "tool_simulation",
        &[
            crate::types::Message::system(system_text, None),
            crate::types::Message::user(user_text, None),
        ],
        Vec::new(),
    )
    .await
    {
        Ok(r) => r,
        // Over budget: the simulation is refused like any other call.
        Err(e) => {
            return Ok(vec![Part::Data(json!({
                "_simulated": true,
                "status": "blocked",
                "message": e.to_string(),
            }))])
        }
    };

    match client.chat().create(request).await {
        Ok(response) => {
            recorder
                .finish_ok(response.usage.as_ref().map(crate::llm::chat_provider_usage))
                .await;
            let text = response
                .choices
                .first()
                .and_then(|c| c.message.content.as_ref())
                .map(|s| s.to_string())
                .unwrap_or_else(|| r#"{"_simulated": true, "status": "ok"}"#.to_string());

            // Try to parse as JSON, fall back to text
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut val) => {
                    // Ensure _simulated flag
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("_simulated".to_string(), json!(true));
                    }
                    Ok(vec![Part::Data(val)])
                }
                Err(_) => Ok(vec![Part::Data(json!({
                    "_simulated": true,
                    "result": text
                }))]),
            }
        }
        Err(e) => {
            tracing::warn!("Tool simulation LLM call failed: {e}");
            recorder.finish_err(&e, None).await;
            Ok(vec![Part::Data(json!({
                "_simulated": true,
                "status": "ok",
                "message": format!("Simulated (LLM unavailable): {}", tool_call.tool_name)
            }))])
        }
    }
}
