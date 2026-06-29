//! Tests for the LLM-facing `invoke_agent` tool. The tool takes a
//! flat `{prompt, agent?, system?}` input and routes through
//! `AgentOrchestrator::invoke()`. These tests pin the wire shape and
//! the orchestrator's hard-coded defaults (Join::Single,
//! ContextScope::Independent, ExecutorHint::Auto, default agent).

use std::sync::Arc;

use distri_types::{Tool, ToolCall};
use serde_json::json;

use crate::agent::ExecutorContext;
use crate::tests::helpers::test_store_config;
use crate::tools::invoke_agent::InvokeAgentTool;
use crate::tools::ExecutorContextTool;
use crate::AgentOrchestratorBuilder;
use distri_types::{RuntimeMode, StandardDefinition};

/// Agent definitions can pin a provider via `model = "<provider>/<model>"`.
/// The parser must split the prefix and set `model_settings.provider`
/// explicitly so `ModelSettings::merge()` doesn't fall back to the
/// workspace's default provider — that fallback was the silent bug
/// where `azure_ai_foundry/gpt-5.4` resolved to qwen.
#[tokio::test]
async fn parse_markdown_resolves_provider_prefix_on_model_settings() {
    let md = r#"---
name = "test_agent"
description = "probe"
[model_settings]
model = "azure_ai_foundry/gpt-5.4"

[tools]
builtin = ["final"]
---
body
"#;
    let def = distri_types::parse_agent_markdown_content(md)
        .await
        .expect("parse ok");
    let ms = def.model_settings.as_ref().expect("model_settings present");
    assert_eq!(
        ms.model, "gpt-5.4",
        "the provider prefix must be stripped from `model`"
    );
    assert!(
        matches!(
            ms.inner.provider,
            distri_types::ModelProvider::AzureAiFoundry { .. }
        ),
        "provider must be set to AzureAiFoundry; got: {:?}",
        ms.inner.provider
    );
}

/// Unknown provider prefixes must error out. The previous behaviour was
/// to silently fall through to OpenAI-compatible, which masked typos
/// like `azure_foundry/gpt-5` (missing `_ai_`).
#[tokio::test]
async fn parse_markdown_rejects_unknown_provider_prefix() {
    let md = r#"---
name = "test_agent"
description = "probe"
[model_settings]
model = "azure_foundry/gpt-5.4"

[tools]
builtin = ["final"]
---
body
"#;
    let err = distri_types::parse_agent_markdown_content(md)
        .await
        .expect_err("unknown prefix must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown model provider prefix") && msg.contains("azure_foundry"),
        "expected unknown-prefix error; got: {msg}"
    );
}

/// Bare model names without a slash keep their default provider — the
/// caller is opting in to "use whatever the workspace default is".
#[tokio::test]
async fn parse_markdown_keeps_bare_model_unchanged() {
    let md = r#"---
name = "test_agent"
description = "probe"
[model_settings]
model = "gpt-4.1-mini"

[tools]
builtin = ["final"]
---
body
"#;
    let def = distri_types::parse_agent_markdown_content(md)
        .await
        .expect("parse ok");
    let ms = def.model_settings.as_ref().expect("model_settings present");
    assert_eq!(ms.model, "gpt-4.1-mini");
}

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

/// invoke_agent advertises a stable name + description + a schema
/// derived from the typed `InvokeAgentInput`. The schema is rigid:
/// `prompt` is required; `agent` and `system` are optional. Nothing
/// else. Pin that here so any regression that re-adds nesting fails
/// loudly.
#[test]
fn invoke_agent_tool_metadata_is_stable() {
    let t = InvokeAgentTool;
    assert_eq!(t.get_name(), "invoke_agent");
    let desc = t.get_description();
    assert!(desc.to_lowercase().contains("sub-agent"));
    let params = t.get_parameters();
    assert_eq!(params["type"], "object");
    assert!(params["properties"]["prompt"].is_object());
    assert!(params["properties"]["agent"].is_object());
    assert!(params["properties"]["system"].is_object());
    let required = params["required"]
        .as_array()
        .expect("schema has `required`")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        required,
        vec!["prompt"],
        "only `prompt` should be required; got: {required:?}"
    );
    // Legacy / orchestrator-controlled fields must NOT appear.
    for forbidden in ["join", "executor", "targets", "context", "message", "tools"] {
        assert!(
            params["properties"][forbidden].is_null(),
            "`{forbidden}` must not appear in the LLM-facing schema; \
             properties: {:?}",
            params["properties"]
        );
    }
    assert!(t.needs_executor_context());
}

/// Happy path with a Named agent. Tool persists exactly one child
/// task whose `agent_name` matches the dispatched agent.
#[tokio::test]
async fn invoke_agent_tool_named_persists_one_child() {
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
        tool_call_id: "tc-named".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "prompt": "go-1",
            "agent": "worker"
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
        1,
        "Named dispatch must persist exactly one child; got {children:?}"
    );
}

/// Ad-hoc dispatch via `system`. The child task uses the agent_id
/// `_adhoc_base` (the AdHoc resolution path).
#[tokio::test]
async fn invoke_agent_tool_adhoc_persists_one_child_under_adhoc_base() {
    use distri_types::stores::TaskStore;

    let orch = build_orch_with_agent("_adhoc_base").await;
    let ctx = parent_ctx(&orch, "_adhoc_base");
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
        tool_call_id: "tc-adhoc".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "prompt": "echo this",
            "system": "You are a leaf worker. Reply with the prompt verbatim then call final."
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
    assert_eq!(children.len(), 1, "expected one child task");
}

/// Hallucinated fields (e.g. `targets`, `context`, `join`, `executor`,
/// `wait`, `message`) are rejected by `deny_unknown_fields` rather
/// than silently dropped. This is the guard that keeps the LLM's
/// mental model aligned with what the orchestrator actually does.
#[tokio::test]
async fn invoke_agent_tool_rejects_hallucinated_fields() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    for forbidden in &[
        "join", "executor", "wait", "targets", "context", "message", "tools",
    ] {
        let mut input = json!({"prompt": "go", "agent": "worker"});
        input[*forbidden] = json!("any-value");

        let tool_call = ToolCall {
            tool_call_id: format!("tc-bad-{forbidden}"),
            tool_name: "invoke_agent".to_string(),
            input,
        };
        let err = InvokeAgentTool
            .execute_with_executor_context(tool_call, ctx.clone())
            .await
            .expect_err(&format!("`{forbidden}` must error"));
        let msg = format!("{err}");
        assert!(
            msg.contains("unknown field") && msg.contains(forbidden),
            "expected `unknown field {forbidden}` deserialise error; got: {msg}"
        );
    }
}

/// Missing `prompt` → typed deserialise error. The schema marks
/// `prompt` as the only required field.
#[tokio::test]
async fn invoke_agent_tool_rejects_missing_prompt() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    let tool_call = ToolCall {
        tool_call_id: "tc-bad".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({"agent": "worker"}),
    };
    let err = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect_err("missing prompt must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("missing field") && msg.contains("prompt"),
        "expected `missing field prompt` error; got: {msg}"
    );
}

/// Empty `prompt` → semantic error from `into_invocation()`. Distinct
/// from the missing-field case: deserialiser succeeds, application
/// logic rejects.
#[tokio::test]
async fn invoke_agent_tool_rejects_empty_prompt() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    let tool_call = ToolCall {
        tool_call_id: "tc-empty".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({"prompt": "   "}),
    };
    let err = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect_err("empty prompt must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("non-empty"),
        "expected `non-empty` error; got: {msg}"
    );
}

/// Passing both `agent` and `system` is rejected: the LLM should pick
/// one path. Allowing both would silently choose one and ignore the
/// other.
#[tokio::test]
async fn invoke_agent_tool_rejects_agent_and_system_together() {
    let orch = build_orch_with_agent("worker").await;
    let ctx = parent_ctx(&orch, "worker");

    let tool_call = ToolCall {
        tool_call_id: "tc-conflict".to_string(),
        tool_name: "invoke_agent".to_string(),
        input: json!({
            "prompt": "go",
            "agent": "worker",
            "system": "be a leaf worker"
        }),
    };
    let err = InvokeAgentTool
        .execute_with_executor_context(tool_call, ctx)
        .await
        .expect_err("agent + system together must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("either") && msg.contains("agent") && msg.contains("system"),
        "expected guidance about choosing one path; got: {msg}"
    );
}
