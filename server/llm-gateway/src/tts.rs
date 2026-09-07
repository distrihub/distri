use crate::tts_types::*;
use bytes::Bytes;
use futures::StreamExt;

/// Call the appropriate TTS provider and return audio bytes.
pub async fn call_tts(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    let span = crate::observability::create_tts_span(
        &req.model,
        &format!("{}", req.provider),
        &req.voice,
        req.response_format.as_str(),
    );
    let _guard = span.enter();
    let start = std::time::Instant::now();

    let result = call_tts_inner(client, req, creds).await;

    crate::observability::record_tts_response(&span, start.elapsed().as_millis() as u64);
    result
}

/// Call the TTS provider and return the audio as a stream of chunks, in
/// playback order, as the provider produces them. OpenAI-compatible endpoints
/// stream by default, ElevenLabs has a `/stream` route, Azure Speech sends a
/// chunked body; DashScope returns a URL and so falls back to one buffered
/// chunk. The content type is known before the first chunk.
pub async fn call_tts_stream(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsStream, String> {
    let span = crate::observability::create_tts_span(
        &req.model,
        &format!("{}", req.provider),
        &req.voice,
        req.response_format.as_str(),
    );
    let _guard = span.enter();
    let start = std::time::Instant::now();

    let result = match provider_request(client, req, creds)? {
        Some((request, fallback_ct)) => send_streaming(request, fallback_ct).await,
        None => {
            // No streaming route for this provider: buffer, then emit once.
            let buffered = call_tts_inner(client, req, creds).await?;
            Ok(TtsStream {
                content_type: buffered.content_type,
                bytes: Box::pin(futures::stream::once(async move {
                    Ok(Bytes::from(buffered.audio))
                })),
            })
        }
    };

    // Time to headers, not to last byte — the stream outlives this call.
    crate::observability::record_tts_response(&span, start.elapsed().as_millis() as u64);
    result
}

async fn call_tts_inner(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    match provider_request(client, req, creds)? {
        Some((request, fallback_ct)) => send_buffered(request, fallback_ct).await,
        None => {
            // Only DashScope lands here: it returns a URL, not audio.
            let base = creds
                .base_url
                .as_deref()
                .unwrap_or("https://dashscope-intl.aliyuncs.com");
            call_dashscope_tts(client, req, base, &creds.api_key).await
        }
    }
}

/// Build the provider request for `req`, with the content type to report when
/// the provider does not say. `None` means the provider has no direct
/// audio-body endpoint (DashScope) and needs its own two-step call.
fn provider_request(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<Option<(reqwest::RequestBuilder, String)>, String> {
    let fallback_ct = req.response_format.content_type().to_string();
    let built = match &req.provider {
        ProviderType::OpenAI => {
            let base = creds
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            openai_compat_request(client, req, base, &creds.api_key)
        }
        ProviderType::Azure => {
            // Azure can do both OpenAI-style TTS and Speech Services TTS.
            // Use azure_region presence to distinguish.
            if creds.region.is_some() {
                azure_speech_request(client, req, creds)
            } else {
                azure_openai_request(client, req, creds)?
            }
        }
        ProviderType::ElevenLabs => elevenlabs_request(client, req, creds),
        ProviderType::AlibabaCloud => return Ok(None),
        ProviderType::AzureAiFoundry | ProviderType::Custom(_) => {
            let base = creds
                .base_url
                .as_deref()
                .ok_or("Base URL is required for this provider")?;
            // Azure AI Foundry endpoints need /openai/v1 appended if missing
            let base = normalize_openai_base(base);
            openai_compat_request(client, req, base, &creds.api_key)
        }
        _ => return Err(format!("TTS not supported for provider: {}", req.provider)),
    };
    Ok(Some((built, fallback_ct)))
}

/// Send and read the whole body.
async fn send_buffered(
    request: reqwest::RequestBuilder,
    fallback_ct: String,
) -> Result<TtsResult, String> {
    let resp = request
        .send()
        .await
        .map_err(|e| format!("TTS request failed: {e}"))?;
    let resp = check_status(resp).await?;
    let content_type = response_content_type(&resp, &fallback_ct);
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read TTS response: {e}"))?;
    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type,
    })
}

/// Send and hand back the body as it arrives.
async fn send_streaming(
    request: reqwest::RequestBuilder,
    fallback_ct: String,
) -> Result<TtsStream, String> {
    let resp = request
        .send()
        .await
        .map_err(|e| format!("TTS request failed: {e}"))?;
    let resp = check_status(resp).await?;
    let content_type = response_content_type(&resp, &fallback_ct);
    let bytes = resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(|e| format!("TTS stream read failed: {e}")));
    Ok(TtsStream {
        content_type,
        bytes: Box::pin(bytes),
    })
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, String> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let err = resp.text().await.unwrap_or_default();
    Err(format!("TTS error ({status}): {err}"))
}

/// The provider's `content-type` when it sends one that names an audio type,
/// otherwise what the requested format implies. Azure Speech and ElevenLabs
/// are known to omit or generalise it.
fn response_content_type(resp: &reqwest::Response, fallback: &str) -> String {
    resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .filter(|ct| ct.starts_with("audio/") || ct.starts_with("application/octet-stream"))
        .map(|ct| ct.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

// ── DashScope TTS (Alibaba Cloud) ────────────────────────────────────────

/// Call the DashScope multimodal-generation API for TTS (non-streaming).
/// Returns a URL in `output.audio.url` which we fetch to get the audio bytes.
async fn call_dashscope_tts(
    client: &reqwest::Client,
    req: &TtsRequest,
    base_url: &str,
    api_key: &str,
) -> Result<TtsResult, String> {
    let url = format!(
        "{}/api/v1/services/aigc/multimodal-generation/generation",
        base_url.trim_end_matches('/')
    );

    let body = serde_json::json!({
        "model": req.model,
        "input": {
            "text": req.input,
            "voice": req.voice,
        }
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("DashScope request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("DashScope TTS error ({status}): {err}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse DashScope response: {e}"))?;

    // DashScope non-streaming returns a presigned URL in output.audio.url
    let audio_url = json
        .get("output")
        .and_then(|o| o.get("audio"))
        .and_then(|a| a.get("url"))
        .and_then(|u| u.as_str())
        .ok_or("No audio URL in DashScope response")?;

    // Fetch the actual audio bytes from the presigned URL
    let audio_resp = client
        .get(audio_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch DashScope audio: {e}"))?;

    if !audio_resp.status().is_success() {
        let status = audio_resp.status();
        let err = audio_resp.text().await.unwrap_or_default();
        return Err(format!(
            "Failed to download DashScope audio ({status}): {err}"
        ));
    }

    let content_type = audio_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/wav")
        .to_string();

    let bytes = audio_resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read DashScope audio: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type,
    })
}

// ── OpenAI-compatible TTS (OpenAI, Azure AI Foundry, custom) ──

/// Request for any OpenAI-compatible endpoint. `base_url` should end with
/// `/v1` or similar — we append `/audio/speech`. The endpoint streams by
/// default, so the same request serves both the buffered and streaming paths.
fn openai_compat_request(
    client: &reqwest::Client,
    req: &TtsRequest,
    base_url: impl AsRef<str>,
    api_key: &str,
) -> reqwest::RequestBuilder {
    let url = format!("{}/audio/speech", base_url.as_ref().trim_end_matches('/'));
    client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&openai_body(req))
}

fn openai_body(req: &TtsRequest) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": req.model,
        "input": req.input,
        "voice": req.voice,
        "response_format": req.response_format.as_str(),
    });
    if let Some(speed) = req.speed {
        body["speed"] = serde_json::json!(speed);
    }
    if let Some(ref instructions) = req.instructions {
        body["instructions"] = serde_json::json!(instructions);
    }
    body
}

// ── Azure OpenAI (deployment-based) ─────────────────────────────────────────

fn azure_openai_request(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<reqwest::RequestBuilder, String> {
    let endpoint = creds
        .base_url
        .as_deref()
        .ok_or("Azure OpenAI endpoint is required")?;
    let deployment = req.azure_deployment.as_deref().unwrap_or(&req.model);
    let base = endpoint
        .trim_end_matches('/')
        .trim_end_matches("/openai/v1")
        .trim_end_matches("/openai")
        .trim_end_matches('/');
    let url = format!(
        "{base}/openai/deployments/{deployment}/audio/speech?api-version=2024-12-01-preview"
    );
    Ok(client
        .post(&url)
        .header("api-key", &creds.api_key)
        .header("Content-Type", "application/json")
        .json(&openai_body(req)))
}

// ── Azure Cognitive Services Speech (SSML) ──────────────────────────────────

fn azure_speech_request(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> reqwest::RequestBuilder {
    let region = creds
        .region
        .as_deref()
        .or(req.azure_region.as_deref())
        .unwrap_or("eastus");
    let url = format!("https://{region}.tts.speech.microsoft.com/cognitiveservices/v1");

    let ssml = format!(
        r#"<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>
            <voice name='{}'>{}</voice>
        </speak>"#,
        req.voice,
        escape_xml(&req.input)
    );

    let output_format = match req.response_format {
        AudioFormat::Mp3 => "audio-24khz-96kbitrate-mono-mp3",
        AudioFormat::Wav => "riff-24khz-16bit-mono-pcm",
        AudioFormat::Opus => "ogg-24khz-16bit-mono-opus",
        _ => "audio-24khz-96kbitrate-mono-mp3",
    };

    client
        .post(&url)
        .header("Ocp-Apim-Subscription-Key", &creds.api_key)
        .header("Content-Type", "application/ssml+xml")
        .header("X-Microsoft-OutputFormat", output_format)
        .body(ssml)
}

// ── ElevenLabs ──────────────────────────────────────────────────────────────

/// ElevenLabs' `/stream` route returns the same audio as the plain route but
/// starts sending before synthesis finishes, so both paths use it.
fn elevenlabs_request(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> reqwest::RequestBuilder {
    let voice_id = req.voice_id.as_deref().unwrap_or("21m00Tcm4TlvDq8ikWAM");
    let base = creds
        .base_url
        .as_deref()
        .unwrap_or("https://api.elevenlabs.io");
    let url = format!(
        "{}/v1/text-to-speech/{voice_id}/stream",
        base.trim_end_matches('/')
    );
    let model_id = req
        .elevenlabs_model_id
        .as_deref()
        .unwrap_or("eleven_multilingual_v2");

    let body = serde_json::json!({
        "text": req.input,
        "model_id": model_id,
    });

    let output_format = match req.response_format {
        AudioFormat::Mp3 => "mp3_44100_128",
        AudioFormat::Pcm => "pcm_44100",
        _ => "mp3_44100_128",
    };

    client
        .post(&url)
        .header("xi-api-key", &creds.api_key)
        .header("Content-Type", "application/json")
        .query(&[("output_format", output_format)])
        .json(&body)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Ensure a base URL ends with `/openai/v1` for Azure AI Foundry endpoints.
/// If the URL already ends with `/v1` or `/openai/v1`, leave it as-is.
/// If it's a bare domain like `https://foo.services.ai.azure.com`, append `/openai/v1`.
fn normalize_openai_base(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") || trimmed.ends_with("/openai/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/openai/v1")
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request(provider: ProviderType) -> TtsRequest {
        serde_json::from_value(serde_json::json!({
            "input": "Hello there. How are you?",
            "model": "gpt-4o-mini-tts",
            "voice": "alloy",
            "provider": provider.as_str(),
            "response_format": "mp3",
        }))
        .expect("request parses")
    }

    async fn collect(stream: TtsStream) -> (String, Vec<u8>, usize) {
        let mut out = Vec::new();
        let mut chunks = 0;
        let mut bytes = stream.bytes;
        while let Some(chunk) = bytes.next().await {
            out.extend_from_slice(&chunk.expect("chunk ok"));
            chunks += 1;
        }
        (stream.content_type, out, chunks)
    }

    #[tokio::test]
    async fn openai_compat_stream_matches_buffered_and_keeps_order() {
        let server = MockServer::start().await;
        let audio: Vec<u8> = (0..=255u8).cycle().take(64 * 1024).collect();
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("Authorization", "Bearer sk-test"))
            .and(body_partial_json(serde_json::json!({
                "model": "gpt-4o-mini-tts", "voice": "alloy", "response_format": "mp3"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(audio.clone()),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let creds = TtsCredentials {
            api_key: "sk-test".into(),
            base_url: Some(format!("{}/v1", server.uri())),
            region: None,
        };
        let req = request(ProviderType::OpenAI);

        let buffered = call_tts(&client, &req, &creds).await.expect("buffered");
        assert_eq!(buffered.content_type, "audio/mpeg");
        assert_eq!(buffered.audio, audio);

        let (ct, streamed, chunks) = collect(
            call_tts_stream(&client, &req, &creds)
                .await
                .expect("stream"),
        )
        .await;
        assert_eq!(ct, "audio/mpeg");
        assert_eq!(
            streamed, audio,
            "chunks reassemble to the same bytes, in order"
        );
        assert!(chunks >= 1);
    }

    #[tokio::test]
    async fn stream_reports_provider_errors_before_any_chunk() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let creds = TtsCredentials {
            api_key: "sk-test".into(),
            base_url: Some(format!("{}/v1", server.uri())),
            region: None,
        };
        let err = call_tts_stream(&client, &request(ProviderType::OpenAI), &creds)
            .await
            .err()
            .expect("error");
        assert!(err.contains("429") && err.contains("slow down"), "{err}");
    }

    #[tokio::test]
    async fn elevenlabs_uses_the_stream_route_and_falls_back_on_content_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/voice-1/stream"))
            .and(header("xi-api-key", "xi-test"))
            .and(query_param("output_format", "mp3_44100_128"))
            .respond_with(
                ResponseTemplate::new(200)
                    // ElevenLabs answers with a generic type; we report the
                    // requested format instead.
                    .insert_header("content-type", "application/json")
                    .set_body_bytes(b"ID3audio".to_vec()),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let creds = TtsCredentials {
            api_key: "xi-test".into(),
            base_url: Some(server.uri()),
            region: None,
        };
        let mut req = request(ProviderType::ElevenLabs);
        req.voice_id = Some("voice-1".into());

        let (ct, streamed, _) = collect(
            call_tts_stream(&client, &req, &creds)
                .await
                .expect("stream"),
        )
        .await;
        assert_eq!(ct, "audio/mpeg");
        assert_eq!(streamed, b"ID3audio");
    }

    #[test]
    fn stream_flag_defaults_to_false_and_parses() {
        let req = request(ProviderType::OpenAI);
        assert!(!req.stream);
        let req: TtsRequest = serde_json::from_value(serde_json::json!({
            "input": "hi", "stream": true
        }))
        .unwrap();
        assert!(req.stream);
    }
}
