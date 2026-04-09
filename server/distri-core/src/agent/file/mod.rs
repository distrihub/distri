use std::{collections::HashMap, sync::Arc};

use distri_types::{filesystem::FileSystemOps, AgentConfig, AgentError, Part, Tool, ToolResponse};
use tokio::sync::RwLock;

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestrator,
};

/// Create artifact tools on-demand from the orchestrator's session filesystem.
/// These are injected as dynamic tools for the artifact agent since they are
/// no longer registered as builtin tools.
fn create_artifact_dynamic_tools(
    orchestrator: &AgentOrchestrator,
) -> Arc<RwLock<Vec<Arc<dyn Tool>>>> {
    let artifact_tools = distri_filesystem::create_artifact_tools(
        orchestrator.session_filesystem.clone() as Arc<dyn FileSystemOps>,
    );
    Arc::new(RwLock::new(artifact_tools))
}

#[cfg(test)]
mod tests;

pub async fn file_agent_defintion() -> Result<distri_types::StandardDefinition, AgentError> {
    parse_agent_markdown_content(include_str!("./artifact_agent.md")).await
}

pub async fn run_file_agent(
    orchestrator: &Arc<AgentOrchestrator>,
    tool_response: crate::types::ToolResponse,
    context: Arc<ExecutorContext>,
    task: &str,
) -> Result<ToolResponse, AgentError> {
    let agent_config = file_agent_defintion().await?;
    // Use parent task's namespace for base_path so artifact agent can access parent's artifacts
    let base_path =
        distri_filesystem::ArtifactWrapper::task_namespace(&context.thread_id, &context.task_id);
    let mut context = context.new_task(&agent_config.name).await;
    context.tool_metadata = Some(HashMap::from([(
        "artifact_base_path".to_string(),
        base_path.to_string().into(),
    )]));
    // Inject artifact tools as dynamic tools so the artifact agent can use them
    context.dynamic_tools = Some(create_artifact_dynamic_tools(orchestrator));
    let result = orchestrator
        .run_inline_agent(
            AgentConfig::StandardAgent(agent_config),
            task,
            Arc::new(context),
        )
        .await?;

    Ok(ToolResponse {
        tool_call_id: tool_response.tool_call_id,
        tool_name: tool_response.tool_name,
        parts: vec![Part::Text(result.content.unwrap_or_default())],
        parts_metadata: None,
    })
}
