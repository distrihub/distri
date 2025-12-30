use std::env;

use crate::{Distri, DistriClientApp, DistriConfig, ToolListItem};
use distri_types::{LLmContext, LlmDefinition, Message, ModelSettings};

#[tokio::test]
async fn live_setup_and_invoke() -> anyhow::Result<()> {
    let Some(ctx) = LiveCtx::new() else {
        eprintln!("skipping live tests; set DISTRI_LIVE_TEST=1 to enable");
        return Ok(());
    };
    ensure_agent(&ctx).await?;
    let client = ctx.client();

    let msg = Message::user("ping".into(), None);
    let resp = client.invoke(&ctx.agent_name, &[msg.clone()]).await?;

    assert!(!resp.is_empty(), "no response from live agent");

    let combined = resp
        .iter()
        .filter_map(|m| m.as_text())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        combined.to_lowercase().contains("echo: ping"),
        "unexpected reply: {combined}"
    );

    Ok(())
}

#[tokio::test]
async fn live_stream() -> anyhow::Result<()> {
    let Some(ctx) = LiveCtx::new() else {
        eprintln!("skipping live tests; set DISTRI_LIVE_TEST=1 to enable");
        return Ok(());
    };
    ensure_agent(&ctx).await?;
    let client = ctx.client();

    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen_clone = seen.clone();
    client
        .invoke_stream(
            &ctx.agent_name,
            &[Message::user("stream".into(), None)],
            move |_| {
                let seen = seen_clone.clone();
                async move {
                    seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            },
        )
        .await?;

    assert!(
        seen.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "no streaming events received"
    );
    Ok(())
}

#[tokio::test]
async fn live_tool_call() -> anyhow::Result<()> {
    let Some(ctx) = LiveCtx::new() else {
        eprintln!("skipping live tests; set DISTRI_LIVE_TEST=1 to enable");
        return Ok(());
    };
    ensure_agent(&ctx).await?;
    let tools = client_app_tools(&ctx.base_url).await?;
    assert!(
        !tools.is_empty(),
        "server /tools returned empty; need at least one tool to run live test"
    );
    let tool = &tools[0];
    let call = distri_types::ToolCall {
        tool_call_id: uuid::Uuid::new_v4().to_string(),
        tool_name: tool.tool_name.clone(),
        input: serde_json::json!({}),
    };

    let client = ctx.client();
    let resp = client.call_tool(&call, None, None).await?;
    assert!(
        resp.is_object() || resp.is_array() || resp.is_string(),
        "unexpected toolcall response shape: {resp}"
    );
    Ok(())
}

#[tokio::test]
async fn live_llm_execute() -> anyhow::Result<()> {
    if env::var("DISTRI_LIVE_LLM").unwrap_or_default() != "1" {
        eprintln!("skipping llm_execute live test; set DISTRI_LIVE_LLM=1 to enable");
        return Ok(());
    }
    let Some(ctx) = LiveCtx::new() else {
        eprintln!("skipping live tests; set DISTRI_LIVE_TEST=1 to enable");
        return Ok(());
    };
    ensure_agent(&ctx).await?;

    let client = ctx.client();
    let llm_def = LlmDefinition {
        name: "live_llm".into(),
        model_settings: ModelSettings {
            model: "gpt-4.1-mini".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let llm_ctx = LLmContext {
        messages: vec![Message::user("say hi".into(), None)],
        ..Default::default()
    };
    let resp = client
        .llm_execute(&llm_def, llm_ctx, vec![], None, false)
        .await?;
    assert!(
        !resp.content.is_empty(),
        "llm_execute returned empty content"
    );
    Ok(())
}

struct LiveCtx {
    base_url: String,
    agent_name: String,
}

impl LiveCtx {
    fn new() -> Option<Self> {
        if env::var("DISTRI_LIVE_TEST").unwrap_or_default() != "1" {
            return None;
        }
        let base_url =
            env::var("DISTRI_BASE_URL").unwrap_or_else(|_| "http://localhost:8081/v1".to_string());
        Some(Self {
            base_url,
            agent_name: format!("distri_agent_test"),
        })
    }

    fn client(&self) -> Distri {
        Distri::from_config(DistriConfig::new(&self.base_url))
    }
}

async fn ensure_agent(ctx: &LiveCtx) -> anyhow::Result<()> {
    let agent = include_str!("./test_agent.md");

    ctx.client().register_agent_markdown(&agent).await?;
    Ok(())
}

async fn client_app_tools(base_url: &str) -> anyhow::Result<Vec<ToolListItem>> {
    let app = DistriClientApp::new(base_url.to_string());
    let tools = app.list_tools().await?;
    Ok(tools)
}
