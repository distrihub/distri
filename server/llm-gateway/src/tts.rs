use crate::tts_types::*;

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

async fn call_tts_inner(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    match &req.provider {
        ProviderType::OpenAI => {
            let base = creds
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            call_openai_compat_tts(client, req, base, &creds.api_key).await
        }
        ProviderType::Azure => {
            // Azure can do both OpenAI-style TTS and Speech Services TTS.
            // Use azure_region presence to distinguish.
            if creds.region.is_some() {
                call_azure_speech(client, req, creds).await
            } else {
                call_azure_openai(client, req, creds).await
            }
        }
        ProviderType::ElevenLabs => call_elevenlabs(client, req, creds).await,
        ProviderType::AlibabaCloud => {
            let base = creds
                .base_url
                .as_deref()
                .unwrap_or("https://dashscope-intl.aliyuncs.com");
            call_dashscope_tts(client, req, base, &creds.api_key).await
        }
        ProviderType::AzureAiFoundry | ProviderType::Custom(_) => {
            let base = creds
                .base_url
                .as_deref()
                .ok_or("Base URL is required for this provider")?;
            // Azure AI Foundry endpoints need /openai/v1 appended if missing
            let base = normalize_openai_base(base);
            call_openai_compat_tts(client, req, base, &creds.api_key).await
        }
        _ => Err(format!("TTS not supported for provider: {}", req.provider)),
    }
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

/// Unified TTS call for any OpenAI-compatible endpoint.
/// `base_url` should end with `/v1` or similar — we append `/audio/speech`.
async fn call_openai_compat_tts(
    client: &reqwest::Client,
    req: &TtsRequest,
    base_url: impl AsRef<str>,
    api_key: &str,
) -> Result<TtsResult, String> {
    let url = format!("{}/audio/speech", base_url.as_ref().trim_end_matches('/'));

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

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("TTS request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("TTS error ({status}): {err}"));
    }

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(req.response_format.content_type())
        .to_string();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read TTS response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: ct,
    })
}

// ── Azure OpenAI (deployment-based) ─────────────────────────────────────────

async fn call_azure_openai(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
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

    let resp = client
        .post(&url)
        .header("api-key", &creds.api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Azure OpenAI request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("Azure OpenAI TTS error ({status}): {err}"));
    }

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(req.response_format.content_type())
        .to_string();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read Azure OpenAI response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: ct,
    })
}

// ── Azure Cognitive Services Speech (SSML) ──────────────────────────────────

async fn call_azure_speech(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
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

    let resp = client
        .post(&url)
        .header("Ocp-Apim-Subscription-Key", &creds.api_key)
        .header("Content-Type", "application/ssml+xml")
        .header("X-Microsoft-OutputFormat", output_format)
        .body(ssml)
        .send()
        .await
        .map_err(|e| format!("Azure Speech request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("Azure Speech TTS error ({status}): {err}"));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read Azure Speech response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: req.response_format.content_type().to_string(),
    })
}

// ── ElevenLabs ──────────────────────────────────────────────────────────────

async fn call_elevenlabs(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    let voice_id = req.voice_id.as_deref().unwrap_or("21m00Tcm4TlvDq8ikWAM");
    let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice_id}");
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

    let resp = client
        .post(&url)
        .header("xi-api-key", &creds.api_key)
        .header("Content-Type", "application/json")
        .query(&[("output_format", output_format)])
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ElevenLabs request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("ElevenLabs TTS error ({status}): {err}"));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read ElevenLabs response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: req.response_format.content_type().to_string(),
    })
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
