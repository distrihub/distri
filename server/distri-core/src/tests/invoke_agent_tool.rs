//! Tests for the LLM-facing `invoke_agent` tool. The tool takes a
//! typed Invocation JSON and routes through `AgentOrchestrator::invoke()`.
//! These tests pin the wire shape (input → typed Invocation
//! deserialize → invoke()) and the response shape (InvocationResult
//! serialized as Part::Data).

use std::sync::Arc;

use distri_types::{Part, Tool, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::tools::invoke_agent::InvokeAgentTool;
use crate::tools::ExecutorContextTool;
use crate::AgentOrchestratorBuilder;
use distri_types::{RuntimeMode, StandardDefinition};

async fn build_orch_with_agent(agent_id: &str) -> Arc<crate::AgentOrchestrator> {
    let orch = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    let def = StandardDefinition {
        name: agent_id.to_string(),
        description: "invoke_agent test".to_string(),
        ..Default::default()
    };
    orch.register_agent_definition(def).await.unwrap();
    orch
}

fn parent_ctx(orch: &Arc<crate::AgentOrchestrator>, agent_id: &str) -> Arc<ExecutorContext> {
    let mut ctx = ExecutorContext::default();
    ctx.agent_id = agent_id.to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.task_id = uuid::Uuid::new_v4().to_string();
    ctx.user_id = "u".to_string();
    ctx.runtime_mode = RuntimeMode::Cli;
    ctx.orchestrator = Some(orch.clone());
    Arc::new(ctx)
}

fn user_message_value(text: &str) -> serde_json::Value {
    // Part is serde-tagged with `part_type` + `data`. Use the typed
    // constructor instead of hand-rolling JSON to avoid drift.
    let msg = distri_types::Message::user(text.to_string(), None);
    serde_json::to_value(&msg).expect("Message serializable")
}

/// invoke_agent advertises a stable name + description + non-empty
/// parameter schema. The schema exposes both the single-dispatch
/// shorthand (`agent` + `message`) and the fan-out form (`targets`).
/// No `join` field — dispatch is always sync; orchestrator infers
/// Single vs All from target count. No `required` either — either
/// shape is accepted.
#[test]
fn invoke_agent_tool_metadata_is_stable() {
    let t = InvokeAgentTool;
    assert_eq!(t.get_name(), "invoke_agent");
    let desc = t.get_description();
    assert!(desc.to_lowercase().contains("sub-agent"));
    let params = t.get_parameters();
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["agent"].is_object());
    assert!(params["properties"]["message"].is_object());
    assert!(params["properties"]["targets"].is_object());
    assert!(
        params["properties"]["join"].is_null(),
        "join must NOT appear in the LLM-facing schema; got: {:?}",
        params["properties"]["join"]
    );
    assert!(t.needs_executor_context());
}

/// Single-dispatch shorthand: the LLM passes `{agent, message}` (no
/// targets array) and the tool internally infers `Join::Single`,
/// persisting exactly one child task. Drives through to the agent
/// loop, which fails with `InvalidConfiguration` because no model is
/// wired in this stripped-down test setup — but the persistence side
/// effect happens BEFORE that, which is what we want to pin.
#[tokio::test]
async fn invoke_agent_tool_single_shorthand_persists_one_child() {
    use distri_types::stores::TaskStore;

    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");
    let parent_task_id = ctx.task_id.clone();
    // The parent task must exist for descendant lookup to find it.
    orch.stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(ctx.thread_id.clone())
                .with_id(parent_task_id.clone()),
        )
        .await
        .expect("seed parent task");

    let tool_call = ToolCall {
        tool_call_id: "tc-single".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "agent": { "type": "named", "agent_id": "worker" },
            "message": user_message_value("go-1")
        }),
    };
    let _ = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await; // Result drives the agent loop; we only care about persistence.

    let descendants = orch
        .stores
        .task_store
        .list_descendant_tasks(&parent_task_id)
        .await
        .unwrap();
    let children: Vec<_> = descendants
        .into_iter()
        .filter(|t| t.id != parent_task_id)
        .collect();
    assert_eq!(
        children.len(),
        1,
        "single dispatch must persist exactly one child; got {children:?}"
    );
}

/// Fan-out form: `targets: [...]` with N entries. Tool internally
/// infers `Join::All` and persists N children before the loops run.
#[tokio::test]
async fn invoke_agent_tool_fanout_form_persists_n_children() {
    use distri_types::stores::TaskStore;

    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");
    let parent_task_id = ctx.task_id.clone();
    orch.stores
        .task_store
        .create_task(
            distri_types::stores::CreateTaskInput::local(ctx.thread_id.clone())
                .with_id(parent_task_id.clone()),
        )
        .await
        .expect("seed parent task");

    let tool_call = ToolCall {
        tool_call_id: "tc-fan".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "context": "independent",
            "targets": [
                {
                    "agent": { "type": "named", "agent_id": "worker" },
                    "message": user_message_value("go-1")
                },
                {
                    "agent": { "type": "named", "agent_id": "worker" },
                    "message": user_message_value("go-2")
                }
            ]
        }),
    };
    let _ = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await;

    let descendants = orch
        .stores
        .task_store
        .list_descendant_tasks(&parent_task_id)
        .await
        .unwrap();
    let children: Vec<_> = descendants
        .into_iter()
        .filter(|t| t.id != parent_task_id)
        .collect();
    assert_eq!(
        children.len(),
        2,
        "fan-out with 2 targets must persist 2 children; got {children:?}"
    );
}

/// Mixing the two shapes (passing both `agent` and `targets`) is
/// rejected with a clear error rather than silently picking one.
#[tokio::test]
async fn invoke_agent_tool_rejects_mixed_input() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    let tool_call = ToolCall {
        tool_call_id: "tc-mixed".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "agent": { "type": "named", "agent_id": "worker" },
            "message": user_message_value("hi"),
            "targets": [
                {
                    "agent": { "type": "named", "agent_id": "worker" },
                    "message": user_message_value("hi-2")
                }
            ]
        }),
    };
    let err = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect_err("mixed shapes must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("single-dispatch") && msg.contains("fan-out"),
        "expected guidance about the two shapes; got: {msg}"
    );
}

/// Empty input → ToolExecution error rather than a panic. The LLM
/// must pass at least one shape's fields.
#[tokio::test]
async fn invoke_agent_tool_rejects_empty_input() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    let tool_call = ToolCall {
        tool_call_id: "tc-bad".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({ "junk": "data" }),
    };
    let err = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect_err("empty input must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("missing"),
        "expected guidance about missing fields; got: {msg}"
    );
}
