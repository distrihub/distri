use crate::{Distri, DistriConfig, LlmExecuteResponse, TtsProvider, TtsSpeechRequest};
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
        model_settings: Some(ModelSettings {
            model: "test".into(),
            inner: Default::default(),
        }),
        tool_format: Default::default(),
        tool_delivery_mode: Default::default(),
    };
    let ctx = LLmContext {
        messages: vec![Message::user("hi".into(), None)],
        ..Default::default()
    };

    let options = crate::LlmExecuteOptions::new(ctx).with_llm_def(llm_def);

    let resp: LlmExecuteResponse = client.llm_execute(options).await.unwrap();

    assert_eq!(resp.finish_reason, "stop");
    assert_eq!(resp.content, "ok");
}

#[tokio::test]
async fn tts_speech_with_defaults() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));

    let resp = client
        .tts_speech(TtsSpeechRequest::new("Hello world"))
        .await
        .unwrap();

    assert!(!resp.audio.is_empty());
    assert_eq!(resp.content_type, "audio/mpeg");
    assert_eq!(resp.provider.as_deref(), Some("openai"));
    assert_eq!(resp.model.as_deref(), Some("tts-1"));
    assert_eq!(resp.voice.as_deref(), Some("alloy"));
}

#[tokio::test]
async fn tts_speech_with_explicit_params() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));

    let resp = client
        .tts_speech(
            TtsSpeechRequest::new("Test")
                .with_model("tts-1-hd")
                .with_voice("nova")
                .with_provider(TtsProvider::AzureOpenai),
        )
        .await
        .unwrap();

    assert!(!resp.audio.is_empty());
    assert_eq!(resp.provider.as_deref(), Some("azure_openai"));
    assert_eq!(resp.model.as_deref(), Some("tts-1-hd"));
    assert_eq!(resp.voice.as_deref(), Some("nova"));
}

#[tokio::test]
async fn tts_models_returns_list() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));

    let resp = client.tts_models().await.unwrap();

    assert_eq!(resp.models.len(), 1);
    assert_eq!(resp.models[0].id, "tts-1");
    assert_eq!(resp.models[0].provider, "openai");
    assert_eq!(resp.models[0].voices.len(), 2);
    assert_eq!(resp.models[0].voices[0].id, "alloy");
    assert!(resp.models[0].voices[0].description.is_some());
    assert!(resp.models[0].formats.contains(&"mp3".to_string()));
}

#[tokio::test]
async fn tts_providers_returns_definitions() {
    let server = spawn_test_server().await;
    let client = Distri::from_config(DistriConfig::new(&server.base_url));

    let providers = client.tts_providers().await.unwrap();

    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].id, "openai");
    assert_eq!(providers[0].label, "OpenAI");
    assert_eq!(providers[0].keys.len(), 1);
    assert_eq!(providers[0].keys[0].key, "OPENAI_API_KEY");
    assert!(providers[0].keys[0].sensitive);
    assert_eq!(providers[0].models.len(), 1);
}

#[test]
fn tts_speech_request_builder() {
    let req = TtsSpeechRequest::new("Hello")
        .with_model("tts-1-hd")
        .with_voice("nova")
        .with_provider(TtsProvider::OpenAI)
        .with_format("wav")
        .with_speed(1.5)
        .with_instructions("Speak softly");

    assert_eq!(req.input, "Hello");
    assert_eq!(req.model.as_deref(), Some("tts-1-hd"));
    assert_eq!(req.voice.as_deref(), Some("nova"));
    assert_eq!(req.provider, Some(TtsProvider::OpenAI));
    assert_eq!(req.response_format.as_deref(), Some("wav"));
    assert_eq!(req.speed, Some(1.5));
    assert_eq!(req.instructions.as_deref(), Some("Speak softly"));
}

#[test]
fn tts_speech_request_minimal_serialization() {
    let req = TtsSpeechRequest::new("Hello world");
    let json = serde_json::to_value(&req).unwrap();

    // Only `input` should be present when no optional fields are set
    assert_eq!(json.get("input").unwrap().as_str(), Some("Hello world"));
    assert!(json.get("model").is_none());
    assert!(json.get("voice").is_none());
    assert!(json.get("provider").is_none());
}

#[test]
fn tts_speech_request_full_serialization() {
    let req = TtsSpeechRequest::new("Hi")
        .with_model("tts-1")
        .with_voice("alloy")
        .with_provider(TtsProvider::OpenAI)
        .with_format("mp3")
        .with_speed(1.0);

    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["input"], "Hi");
    assert_eq!(json["model"], "tts-1");
    assert_eq!(json["voice"], "alloy");
    assert_eq!(json["provider"], "openai");
    assert_eq!(json["response_format"], "mp3");
    assert_eq!(json["speed"], 1.0);
}

async fn spawn_test_server() -> TestServer {
    let server = HttpServer::new(|| {
        App::new()
            .route("/agents/{id}", web::post().to(agent_handler))
            .route("/tools/call", web::post().to(tool_handler))
            .route("/llm/execute", web::post().to(llm_handler))
            .route("/audio/speech", web::post().to(tts_speech_handler))
            .route("/audio/models", web::get().to(tts_models_handler))
            .route("/audio/providers", web::get().to(tts_providers_handler))
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
        "usage": { "input_tokens": messages as u32, "output_tokens": 0, "total_tokens": messages as u32 },
    }))
}

async fn tts_speech_handler(body: web::Json<serde_json::Value>) -> HttpResponse {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("tts-1");
    let voice = body
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("alloy");
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openai");

    // Return fake audio bytes
    let audio = b"fake-audio-bytes";

    HttpResponse::Ok()
        .content_type("audio/mpeg")
        .insert_header(("X-TTS-Provider", provider))
        .insert_header(("X-TTS-Model", model))
        .insert_header(("X-TTS-Voice", voice))
        .body(audio.to_vec())
}

async fn tts_models_handler() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "models": [
            {
                "id": "tts-1",
                "provider": "openai",
                "name": "TTS-1",
                "voices": [
                    { "id": "alloy", "name": "Alloy", "description": "Neutral and balanced" },
                    { "id": "nova", "name": "Nova", "description": "Bright and energetic" }
                ],
                "formats": ["mp3", "wav", "opus"]
            }
        ]
    }))
}

async fn tts_providers_handler() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!([
        {
            "id": "openai",
            "label": "OpenAI",
            "keys": [{ "key": "OPENAI_API_KEY", "label": "API key", "placeholder": "sk-...", "required": true, "sensitive": true }],
            "models": [
                {
                    "id": "tts-1",
                    "provider": "openai",
                    "name": "TTS-1",
                    "voices": [{ "id": "alloy", "name": "Alloy" }],
                    "formats": ["mp3"]
                }
            ]
        }
    ]))
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
