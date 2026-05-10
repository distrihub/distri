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
/// parameter schema. These are part of the public LLM-facing
/// contract; renaming the tool is a breaking change for every agent
/// definition that lists `invoke_agent` in `tools.builtin`.
#[test]
fn invoke_agent_tool_metadata_is_stable() {
    let t = InvokeAgentTool;
    assert_eq!(t.get_name(), "invoke_agent");
    let desc = t.get_description();
    assert!(desc.contains("Invocation"));
    let params = t.get_parameters();
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["targets"].is_object());
    assert!(params["properties"]["join"].is_object());
    assert!(params["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "targets"));
    assert!(t.needs_executor_context());
}

/// Detached + Local round-trips through the tool: typed Invocation
/// arrives, returns InvocationResult::TaskIds with the addressable
/// task_ids. Persistence is verified through the store side-channel.
#[tokio::test]
async fn invoke_agent_tool_routes_detached_local() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");
    let parent_task_id = ctx.task_id.clone();

    let tool_call = ToolCall {
        tool_call_id: "tc-1".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "join": "detached",
            "context": "independent",
            "executor": { "kind": "auto" },
            "tools": { "kind": "inherit" },
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
    let parts = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect("invoke_agent ok");
    let data = parts
        .iter()
        .find_map(|p| match p {
            Part::Data(v) => Some(v),
            _ => None,
        })
        .expect("Part::Data response");
    assert_eq!(data["kind"], "task_ids");
    let task_ids: Vec<&str> = data["task_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(task_ids.len(), 2);

    // Each task_id is addressable in the store with parent linkage.
    for tid in &task_ids {
        let row = orch
            .stores
            .task_store
            .get_task(tid)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(row.parent_task_id.as_deref(), Some(parent_task_id.as_str()));
    }
}

/// Malformed Invocation input → ToolExecution error rather than a
/// panic. Validates that the deserializer error path is the one the
/// LLM-facing tool surfaces.
#[tokio::test]
async fn invoke_agent_tool_rejects_malformed_input() {
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
        .expect_err("malformed input must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid Invocation") || msg.contains("missing field"),
        "expected deserializer error; got: {msg}"
    );
}
