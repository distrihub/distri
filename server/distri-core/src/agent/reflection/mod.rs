use distri_types::{AgentConfig, AgentError};
use serde_json::Value;
use std::sync::Arc;

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestrator,
};

/// Result of a reflection agent run, containing both the text content
/// and the structured tool call result from the `reflect` tool.
pub struct ReflectionResult {
    /// Text content from the reflection agent's response
    pub content: String,
    /// Structured result from the `reflect` tool call (contains quality, completeness, should_continue)
    pub final_result: Option<Value>,
}

pub async fn reflection_agent_definition() -> Result<distri_types::StandardDefinition, AgentError> {
    parse_agent_markdown_content(include_str!("./reflection_agent.md")).await
}

pub async fn run_reflection_agent(
    orchestrator: &Arc<AgentOrchestrator>,
    context: Arc<ExecutorContext>,
    task: &str,
    execution_history: &str,
) -> Result<ReflectionResult, AgentError> {
    let agent_config = reflection_agent_definition().await?;
    let mut new_context = context.new_task(&agent_config.name).await;

    // Set verbose to false to suppress reflection output when everything is fine
    new_context.verbose = false;

    // The execution history will be passed in the task description for the reflection agent to analyze
    let task_with_history = format!(
        "{}\n\n## Execution History to Analyze:\n{}",
        task, execution_history
    );

    let new_context = Arc::new(new_context);
    let new_context_clone = new_context.clone();

    let result = orchestrator
        .run_inline_agent(
            AgentConfig::StandardAgent(agent_config),
            &task_with_history,
            new_context,
        )
        .await?;

    // The reflect tool stores its structured result as the final result in the child context
    let final_result = new_context_clone.get_final_result().await;

    Ok(ReflectionResult {
        content: result.content.unwrap_or_default(),
        final_result,
    })
}
