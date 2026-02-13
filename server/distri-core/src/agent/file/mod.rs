use std::{collections::HashMap, sync::Arc};

use distri_types::{AgentConfig, AgentError, Part, ToolResponse};

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestrator,
};

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
/// Process tool response using FileSystem's ArtifactWrapper for large content
/// Automatically write them to a file, run an inline artifact_agent to analyze, and return short summary + artifact references
pub async fn process_large_tool_responses(
    tool_response: crate::types::ToolResponse,
    thread_id: &str,
    task_id: &str,
    orchestrator: &Arc<crate::AgentOrchestrator>,
    original_task: &str,
) -> Result<crate::types::ToolResponse, AgentError> {
    // Create artifact wrapper with base path
    let artifact_base_path = distri_filesystem::ArtifactWrapper::task_namespace(thread_id, task_id);
    let artifact_wrapper = orchestrator
        .session_filesystem
        .create_artifact_wrapper(artifact_base_path.clone())
        .await
        .map_err(|e| AgentError::ToolResponseProcessing(e.to_string()))?;

    // Use ArtifactWrapper to process the tool response (writes to artifacts)
    let production_config = distri_filesystem::ArtifactStorageConfig::for_production();

    let processed_response = artifact_wrapper
        .process_tool_response(
            distri_types::ToolResponse {
                tool_call_id: tool_response.tool_call_id.clone(),
                tool_name: tool_response.tool_name.clone(),
                parts: tool_response.parts.clone(),
                parts_metadata: None,
            },
            &production_config,
        )
        .await
        .map_err(|e| AgentError::ToolResponseProcessing(e.to_string()))?;

    // Check if any artifacts were created (indicated by Artifact parts)
    let has_artifacts = processed_response
        .parts
        .iter()
        .any(|part| matches!(part, distri_types::Part::Artifact(_)));

    if !has_artifacts {
        // No artifacts created, return original response
        return Ok(crate::types::ToolResponse {
            tool_call_id: processed_response.tool_call_id,
            tool_name: processed_response.tool_name,
            parts: processed_response.parts,
            parts_metadata: None,
        });
    }

    // Artifacts were created - now immediately invoke file_agent to analyze them
    tracing::info!("🔄 Artifacts created, invoking file_agent for immediate analysis");

    // Use the existing orchestrator - no need to create a new one
    use crate::agent::ExecutorContext;
    use std::collections::HashMap;

    let file_agent_context = Arc::new(ExecutorContext {
        thread_id: thread_id.to_string(),
        task_id: format!("{}_file_analysis", task_id),
        orchestrator: Some(orchestrator.clone()),
        tool_metadata: Some(HashMap::from([(
            "artifact_base_path".to_string(),
            artifact_base_path.to_string().into(),
        )])),
        ..Default::default()
    });

    // Extract artifact filenames from the processed response
    let artifact_filenames: Vec<String> = processed_response
        .parts
        .iter()
        .filter_map(|part| {
            if let distri_types::Part::Artifact(metadata) = part {
                Some(metadata.file_id.clone())
            } else {
                None
            }
        })
        .collect();

    // Get the file agent definition and run it
    let agent_config = file_agent_defintion().await?;
    let analysis_task = format!(
        "Original task: {}\n\nAnalyze artifact file(s): {}.\n\nRead each file in chunks (start with lines 1-100) using read_artifact with start_line/end_line parameters. Provide a concise summary of key findings related to the original task. Call final() immediately after getting enough information.",
        original_task,
        artifact_filenames.join(", ")
    );

    let analysis_result = orchestrator
        .run_inline_agent(
            AgentConfig::StandardAgent(agent_config),
            &analysis_task,
            file_agent_context,
        )
        .await
        .map_err(|e| {
            AgentError::ToolResponseProcessing(format!("File agent analysis failed: {}", e))
        })?;

    // Combine the analysis result with artifact references
    let mut result_parts = Vec::new();

    // Add the analysis summary as text
    if let Some(analysis_content) = analysis_result.content {
        if !analysis_content.trim().is_empty() {
            result_parts.push(distri_types::Part::Text(analysis_content));
        }
    }

    // Count artifacts before moving
    let artifact_count = processed_response
        .parts
        .iter()
        .filter(|part| matches!(part, distri_types::Part::Artifact(_)))
        .count();

    // Add the artifact references for further investigation
    result_parts.extend(
        processed_response
            .parts
            .into_iter()
            .filter(|part| matches!(part, distri_types::Part::Artifact(_))),
    );

    // If no meaningful analysis was produced, provide a fallback summary
    if result_parts.is_empty()
        || !result_parts
            .iter()
            .any(|part| matches!(part, distri_types::Part::Text(_)))
    {
        result_parts.insert(0, distri_types::Part::Text(
            format!("Large content from {} has been stored as {} artifact(s) and is available for detailed analysis using artifact tools.", 
                    tool_response.tool_name,
                    artifact_count)
        ));
    }

    tracing::info!("✅ File agent analysis complete, returning summary + artifact references");

    Ok(crate::types::ToolResponse {
        tool_call_id: tool_response.tool_call_id,
        tool_name: tool_response.tool_name,
        parts: result_parts,
        parts_metadata: None,
    })
}
