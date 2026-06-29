use distri_types::ToolContext;

use crate::agent::ExecutorContext;

pub fn to_tool_context(executor_context: &ExecutorContext) -> ToolContext {
    // Use the orchestrator's existing session store directly
    let session_store = executor_context
        .orchestrator
        .as_ref()
        .map(|orch| orch.stores.session_store.clone())
        .expect("Orchestrator should have a session store");

    ToolContext {
        agent_id: executor_context.agent_id.clone(),
        session_id: executor_context.session_id.clone(),
        task_id: executor_context.task_id.clone(),
        run_id: executor_context.run_id.clone(),
        thread_id: executor_context.thread_id.clone(),
        user_id: executor_context.user_id.clone(),
        session_store,
        event_tx: executor_context.event_tx.clone(),
        metadata: executor_context.tool_metadata.clone(),
    }
}
