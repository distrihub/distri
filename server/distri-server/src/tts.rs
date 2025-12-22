use actix_web::{web, HttpResponse};
use base64::Engine;
use distri_core::voice::{StreamingConfig, TranscribeRequest, TtsConfig, TtsRequest, TtsService};
use serde_json;

// HTTP handlers
pub async fn synthesize_tts(
    req: web::Json<TtsRequest>,
    tts_service: web::Data<TtsService>,
) -> HttpResponse {
    let request = req.into_inner();

    match tts_service.synthesize(&request).await {
        Ok(audio_bytes) => HttpResponse::Ok()
            .content_type("audio/mpeg")
            .insert_header(("Content-Disposition", "attachment; filename=\"speech.mp3\""))
            .body(audio_bytes.to_vec()),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("TTS synthesis failed: {}", e)
        })),
    }
}

pub async fn get_available_voices(tts_service: web::Data<TtsService>) -> HttpResponse {
    let voices = tts_service.get_available_voices();
    HttpResponse::Ok().json(voices)
}

pub async fn transcribe_speech(
    req: web::Json<serde_json::Value>,
    tts_service: web::Data<TtsService>,
) -> HttpResponse {
    // For now, expect JSON with base64 encoded audio
    let json_req = req.into_inner();

    let audio_data = match json_req.get("audio") {
        Some(serde_json::Value::String(base64_audio)) => {
            match base64::engine::general_purpose::STANDARD.decode(base64_audio) {
                Ok(data) => data,
                Err(_) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "Invalid base64 audio data"
                    }));
                }
            }
        }
        _ => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Missing or invalid audio field"
            }));
        }
    };

    let model = json_req
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let language = json_req
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let temperature = json_req
        .get("temperature")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32);

    let transcribe_request = TranscribeRequest {
        model,
        language,
        temperature,
    };

    match tts_service
        .transcribe_with_options(&audio_data, &transcribe_request)
        .await
    {
        Ok(text) => HttpResponse::Ok().json(serde_json::json!({
            "text": text
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Transcription failed: {}", e)
        })),
    }
}

// New streaming endpoints
pub async fn start_streaming_session() -> HttpResponse {
    // Create a new streaming session
    let config = TtsConfig::from_env();

    if config.openai_api_key.is_none() {
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "OpenAI API key not configured for streaming"
        }));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let streaming_config = StreamingConfig::default();

    // In a real implementation, you'd store the session for later use
    HttpResponse::Ok().json(serde_json::json!({
        "session_id": session_id,
        "config": streaming_config
    }))
}

pub async fn websocket_voice_stream(
    _req: actix_web::HttpRequest,
    _stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    // This would implement WebSocket upgrade for real-time voice streaming
    // For now, return a placeholder
    Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
        "error": "WebSocket streaming not implemented in this handler. Use the realtime voice client directly."
    })))
}
