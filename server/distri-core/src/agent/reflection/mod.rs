use distri_types::{AgentConfig, AgentError};
use std::sync::Arc;

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestrator,
};

pub async fn reflection_agent_definition() -> Result<distri_types::StandardDefinition, AgentError> {
    parse_agent_markdown_content(include_str!("./reflection_agent.md")).await
}

pub async fn run_reflection_agent(
    orchestrator: &Arc<AgentOrchestrator>,
    context: Arc<ExecutorContext>,
    task: &str,
    execution_history: &str,
) -> Result<String, AgentError> {
    let agent_config = reflection_agent_definition().await?;
    let mut new_context = context.new_task(&agent_config.name).await;

    // Set verbose to false to suppress reflection output when everything is fine
    new_context.verbose = false;

    // The execution history will be passed in the task description for the reflection agent to analyze
    let task_with_history = format!(
        "{}\n\n## Execution History to Analyze:\n{}",
        task, execution_history
    );

    let result = orchestrator
        .run_inline_agent(
            AgentConfig::StandardAgent(agent_config),
            &task_with_history,
            Arc::new(new_context),
        )
        .await?;

    Ok(result.content.unwrap_or_default())
}
