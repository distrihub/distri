//! Span end times cover the work they time.
//!
//! tracing-opentelemetry (0.29) stamps a span's OTel end time on every *exit*
//! of the span, not when it closes. A streamed LLM call whose span was entered
//! only while the stream opened exported a 12–39 ms duration; a step span
//! entered with `in_scope` to parent a tool span ended when the tool started.
//! These tests run the real tracing-opentelemetry layer and read the end time
//! it would export (`OtelData.builder.end_time`) as the span closes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use distri_types::{
    AgentEvent, AgentEventType, LlmDefinition, Message, ModelProvider, ModelSettings,
    ModelSettingsInner, Part, ToolCallFormat, ToolResponse,
};
use tracing::span;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::agent::hooks::otel::OtelHooks;
use crate::agent::types::AgentHooks;
use crate::agent::ExecutorContext;
use crate::llm::LLMExecutor;

/// `(tracing span name, exported duration)` of every span that closed.
#[derive(Clone, Default)]
struct Durations(Arc<Mutex<Vec<(String, Duration)>>>);

impl Durations {
    fn of(&self, name: &str) -> Vec<Duration> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, d)| *d)
            .collect()
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Durations {
    // Layered below the OTel layer, so this runs before it takes `OtelData`.
    fn on_close(&self, id: span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("span");
        let ext = span.extensions();
        let Some(data) = ext.get::<tracing_opentelemetry::OtelData>() else {
            return;
        };
        let start = data.builder.start_time.expect("start time");
        // No end yet: the SDK ends the span when it is built, i.e. now.
        let end = data.builder.end_time.unwrap_or_else(SystemTime::now);
        let took = end.duration_since(start).unwrap_or_default();
        self.0.lock().unwrap().push((span.name().to_string(), took));
    }
}

fn capture() -> (Durations, tracing::subscriber::DefaultGuard) {
    let durations = Durations::default();
    let subscriber = tracing_subscriber::registry()
        .with(durations.clone())
        .with(tracing_opentelemetry::layer());
    let guard = tracing::subscriber::set_default(subscriber);
    (durations, guard)
}

const STREAM_GAP: Duration = Duration::from_millis(300);

/// An OpenAI-compatible endpoint that sends the stream's headers and first
/// chunk at once, then waits `STREAM_GAP` before the rest — a model still
/// generating.
async fn slow_stream_model() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 65536];
                let _ = sock.read(&mut buf).await;
                let chunk = |delta: serde_json::Value, finish: serde_json::Value| {
                    let v = serde_json::json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 0,
                        "model": "stub",
                        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
                    });
                    format!("data: {v}\n\n")
                };
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                            cache-control: no-cache\r\nconnection: close\r\n\r\n";
                let first = chunk(
                    serde_json::json!({"role": "assistant", "content": "Hel"}),
                    serde_json::Value::Null,
                );
                let _ = sock.write_all(format!("{head}{first}").as_bytes()).await;
                let _ = sock.flush().await;
                tokio::time::sleep(STREAM_GAP).await;
                let usage = serde_json::json!({
                    "id": "c", "object": "chat.completion.chunk", "created": 0,
                    "model": "stub", "choices": [],
                    "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15},
                });
                let rest = format!(
                    "{}{}data: {usage}\n\ndata: [DONE]\n\n",
                    chunk(
                        serde_json::json!({"content": "lo"}),
                        serde_json::Value::Null
                    ),
                    chunk(serde_json::json!({}), serde_json::json!("stop")),
                );
                let _ = sock.write_all(rest.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    url
}

/// The LLM span of a streamed call lasts until the stream has been read, not
/// just until it opened.
#[tokio::test]
async fn streamed_llm_span_covers_the_whole_stream() {
    let url = slow_stream_model().await;
    let (durations, _guard) = capture();

    let ctx = Arc::new(ExecutorContext::default());
    let executor = LLMExecutor::new(
        LlmDefinition {
            name: "timing".to_string(),
            model_settings: Some(ModelSettings {
                model: "stub".to_string(),
                inner: ModelSettingsInner {
                    provider: ModelProvider::OpenAICompatible {
                        base_url: url,
                        api_key: Some("test-key".to_string()),
                        project_id: None,
                    },
                    ..Default::default()
                },
            }),
            tool_format: ToolCallFormat::Provider,
            tool_delivery_mode: Default::default(),
        },
        vec![],
        ctx.clone(),
        None,
        None,
    );
    let result = executor
        .execute_stream(&[Message::user("hi".to_string(), None)], ctx)
        .await
        .expect("stream");
    assert_eq!(result.content, "Hello");
    drop(executor);

    let llm = durations.of("gen_ai.chat");
    assert_eq!(llm.len(), 1, "one LLM span: {llm:?}");
    assert!(
        llm[0] >= STREAM_GAP,
        "LLM span lasted {:?}, the stream took at least {STREAM_GAP:?}",
        llm[0]
    );
}

fn event(event: AgentEventType) -> AgentEvent {
    AgentEvent {
        timestamp: chrono::Utc::now(),
        thread_id: "t1".to_string(),
        run_id: "r1".to_string(),
        task_id: "task1".to_string(),
        parent_task_id: None,
        child: None,
        agent_id: "coder".to_string(),
        user_id: None,
        identifier_id: None,
        workspace_id: None,
        channel_id: None,
        event,
    }
}

/// A step span entered to parent its tool span still ends when the step
/// completes, not when the tool started.
#[tokio::test]
async fn step_span_ends_when_the_step_completes() {
    let (durations, _guard) = capture();
    let hooks = OtelHooks::default();
    let work = Duration::from_millis(200);

    let send = |e: AgentEventType| {
        let hooks = &hooks;
        async move { hooks.on_event(&event(e)).await.unwrap() }
    };
    send(AgentEventType::StepStarted {
        step_id: "s1".to_string(),
        step_index: 0,
    })
    .await;
    send(AgentEventType::ToolExecutionStart {
        step_id: "s1".to_string(),
        tool_call_id: "tc-1".to_string(),
        tool_call_name: "bash".to_string(),
        input: serde_json::json!({"cmd": "sleep"}),
    })
    .await;
    tokio::time::sleep(work).await;
    send(AgentEventType::ToolResults {
        step_id: "s1".to_string(),
        parent_message_id: None,
        results: vec![ToolResponse {
            tool_call_id: "tc-1".to_string(),
            tool_name: "bash".to_string(),
            parts: vec![Part::Text("done".to_string())],
            parts_metadata: None,
        }],
    })
    .await;
    send(AgentEventType::StepCompleted {
        step_id: "s1".to_string(),
        success: true,
        context_budget: None,
        usage: None,
    })
    .await;

    let step = durations.of("gen_ai.step");
    assert_eq!(step.len(), 1, "one step span: {step:?}");
    assert!(
        step[0] >= work,
        "step span lasted {:?}, its tool took at least {work:?}",
        step[0]
    );
    let tool = durations.of("gen_ai.execute_tool");
    assert_eq!(tool.len(), 1);
    assert!(tool[0] >= work, "tool span lasted {:?}", tool[0]);
}
