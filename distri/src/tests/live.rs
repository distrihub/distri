use std::env;

use crate::{Distri, DistriClientApp, DistriConfig, ToolListItem};
use distri_types::{LLmContext, LlmDefinition, Message, ModelSettings};
use serde::Deserialize;

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

// ============================================================================
// Usage Recording Tests
// ============================================================================

/// Response from /v1/usage endpoint
#[derive(Debug, Deserialize)]
struct UsageResponse {
    current: CurrentUsage,
}

#[derive(Debug, Deserialize)]
struct CurrentUsage {
    day_tokens: i64,
    month_tokens: i64,
}

/// Smoke test: Make an agent call and verify usage is recorded
///
/// This test:
/// 1. Gets initial usage stats
/// 2. Makes an agent call (which should trigger LLM usage)
/// 3. Waits briefly for async usage recording
/// 4. Gets usage stats again
/// 5. Verifies tokens increased
#[tokio::test]
async fn live_usage_recording() -> anyhow::Result<()> {
    if env::var("DISTRI_LIVE_USAGE_TEST").unwrap_or_default() != "1" {
        eprintln!("skipping usage recording test; set DISTRI_LIVE_USAGE_TEST=1 to enable");
        return Ok(());
    }

    let Some(ctx) = LiveCtx::new() else {
        eprintln!("skipping live tests; set DISTRI_LIVE_TEST=1 to enable");
        return Ok(());
    };

    // Need an API key for authenticated requests
    let api_key = env::var("DISTRI_API_KEY").ok();
    if api_key.is_none() {
        eprintln!("skipping usage test; DISTRI_API_KEY required");
        return Ok(());
    }

    ensure_agent(&ctx).await?;

    let api_key = api_key.unwrap();
    let config = DistriConfig::new(&ctx.base_url).with_api_key(&api_key);
    let client = Distri::from_config(config);
    let http = reqwest::Client::new();

    // Get initial usage
    let initial_usage = get_usage(&http, &ctx.base_url, &api_key).await?;
    eprintln!(
        "Initial usage - day_tokens: {}, month_tokens: {}",
        initial_usage.current.day_tokens, initial_usage.current.month_tokens
    );

    // Make an agent call that will use tokens
    let msg = Message::user("What is 2 + 2? Reply with just the number.".into(), None);
    let resp = client.invoke(&ctx.agent_name, &[msg]).await?;

    assert!(!resp.is_empty(), "no response from agent");
    eprintln!("Agent response received");

    // Wait for async usage recording to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Get updated usage
    let final_usage = get_usage(&http, &ctx.base_url, &api_key).await?;
    eprintln!(
        "Final usage - day_tokens: {}, month_tokens: {}",
        final_usage.current.day_tokens, final_usage.current.month_tokens
    );

    // Verify tokens increased
    assert!(
        final_usage.current.month_tokens > initial_usage.current.month_tokens,
        "Usage should have increased after agent call. Initial: {}, Final: {}",
        initial_usage.current.month_tokens,
        final_usage.current.month_tokens
    );

    eprintln!(
        "Usage recording verified! Tokens used: {}",
        final_usage.current.month_tokens - initial_usage.current.month_tokens
    );

    Ok(())
}

async fn get_usage(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<UsageResponse> {
    let url = format!("{}/usage", base_url);
    let resp = http
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get usage ({}): {}", status, body);
    }

    let usage: UsageResponse = resp.json().await?;
    Ok(usage)
}
