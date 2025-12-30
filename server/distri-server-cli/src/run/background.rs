use distri_core::agent::AgentOrchestrator;
use distri_core::types::Message;
use std::sync::Arc;

use crate::run::printer::run_stream_with_printer;
use crate::tool_renderers::ToolRendererRegistry;

pub async fn run(
    agent_name: &str,
    executor: Arc<AgentOrchestrator>,
    task: Message,
    verbose: bool,
    user_id: Option<&str>,
    tool_renderers: Option<Arc<ToolRendererRegistry>>,
) -> anyhow::Result<()> {
    // Run in non-interactive mode (background)
    let _ = run_stream_with_printer(
        agent_name,
        executor,
        task,
        verbose,
        None,
        None,
        user_id,
        tool_renderers,
    )
    .await?;

    Ok(())
}
