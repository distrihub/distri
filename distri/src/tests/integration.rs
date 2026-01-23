use crate::{Distri, DistriConfig, LlmExecuteResponse};
use actix_web::{App, HttpResponse, HttpServer, dev::ServerHandle, web};
use distri_a2a::{EventKind, Message as A2aMessage, MessageKind, Part as A2aPart, TextPart};
use distri_types::{
    LLmContext, LlmDefinition, Message, MessageRole, ModelSettings, Part, ToolCall,
};

#[tokio::test]
async fn invoke_returns_distri_messages() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));

    let messages = vec![Message {
        role: MessageRole::User,
        parts: vec![Part::Text("hello".into())],
        ..Default::default()
    }];

    let resp = client.invoke("test-agent", &messages).await.unwrap();
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0].as_text().as_deref(), Some("hi"));
}

#[tokio::test]
async fn invoke_stream_yields_events() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));
    let messages = vec![Message::user("stream me".into(), None)];

    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen_clone = seen.clone();
    client
        .invoke_stream("test-agent", &messages, move |_| {
            let seen = seen_clone.clone();
            async move {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .await
        .unwrap();

    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn call_tool_round_trips() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));
    let tool_call = ToolCall {
        tool_call_id: "abc".into(),
        tool_name: "echo".into(),
        input: serde_json::json!({"msg": "hi"}),
    };

    let resp = client
        .call_tool(&tool_call, Some("sess-1".into()), None)
        .await
        .unwrap();

    assert_eq!(resp.get("tool_name").and_then(|v| v.as_str()), Some("echo"));
}

#[tokio::test]
async fn llm_execute_returns_payload() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));
    let llm_def = LlmDefinition {
        name: "unit-llm".into(),
        model_settings: ModelSettings {
            model: "test".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = LLmContext {
        messages: vec![Message::user("hi".into(), None)],
        ..Default::default()
    };

    let options = crate::LlmExecuteOptions::new(ctx)
        .with_llm_def(llm_def);

    let resp: LlmExecuteResponse = client
        .llm_execute(options)
        .await
        .unwrap();

    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.content, "ok");
}

async fn spawn_test_server() -> TestServer {
    let server = HttpServer::new(|| {
        App::new()
            .route("/agents/{id}", web::post().to(agent_handler))
            .route("/tools/call", web::post().to(tool_handler))
            .route("/llm/execute", web::post().to(llm_handler))
    })
    .bind(("127.0.0.1", 0))
    .unwrap();

    let port = server.addrs()[0].port();
    let handle = server.run();
    let server_handle = handle.handle();

    tokio::spawn(async move {
        let _ = handle.await;
    });

    TestServer {
        base_url: format!("http://127.0.0.1:{port}"),
        handle: server_handle,
    }
}

async fn agent_handler(body: web::Bytes) -> HttpResponse {
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::BadRequest().body(format!("invalid json: {e}"));
        }
    };
    let method = payload
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default();

    // Build a minimal A2A message to return.
    let mk = MessageKind::Message(A2aMessage {
        kind: EventKind::Message,
        message_id: "mid".into(),
        role: distri_a2a::Role::Agent,
        parts: vec![A2aPart::Text(TextPart { text: "hi".into() })],
        context_id: None,
        task_id: None,
        reference_task_ids: vec![],
        extensions: vec![],
        metadata: None,
    });

    if method == "message/stream" {
        let rpc = serde_json::json!({
            "jsonrpc": "2.0",
            "result": mk,
            "id": "1"
        });
        let body = format!("data: {}\n\n", serde_json::to_string(&rpc).unwrap());
        return HttpResponse::Ok()
            .insert_header(("content-type", "text/event-stream"))
            .body(body);
    }

    let rpc = serde_json::json!({
        "jsonrpc": "2.0",
        "result": [mk],
        "id": "1"
    });
    HttpResponse::Ok().json(rpc)
}

async fn tool_handler(body: web::Json<serde_json::Value>) -> HttpResponse {
    let tool_name = body
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "tool_name": tool_name
    }))
}

async fn llm_handler(body: web::Json<serde_json::Value>) -> HttpResponse {
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|m| m.len())
        .unwrap_or(0);
    HttpResponse::Ok().json(serde_json::json!({
        "finish_reason": "stop",
        "content": "ok",
        "tool_calls": [],
        "token_usage": messages as u32,
    }))
}

struct TestServer {
    base_url: String,
    handle: ServerHandle,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            let _ = handle.stop(true).await;
        });
    }
}
