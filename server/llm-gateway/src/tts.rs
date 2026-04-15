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
        ProviderType::OpenAI => call_openai(client, req, creds).await,
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
        ProviderType::AlibabaCloud => call_alibaba_cloud_tts(client, req, creds).await,
        _ => Err(format!("TTS not supported for provider: {}", req.provider)),
    }
}

// ── OpenAI ──────────────────────────────────────────────────────────────────

async fn call_openai(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    let base = creds
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");
    let url = format!("{}/v1/audio/speech", base);

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
        .header("Authorization", format!("Bearer {}", creds.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI TTS error ({status}): {err}"));
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
        .map_err(|e| format!("Failed to read OpenAI response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: ct,
    })
}

// ── Azure OpenAI ────────────────────────────────────────────────────────────

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
    // Strip common suffixes that users may include in the endpoint URL
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

// ── Azure Cognitive Services Speech ─────────────────────────────────────────

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

// ── Alibaba Cloud (DashScope) ────────────────────────────────────────────────

async fn call_alibaba_cloud_tts(
    client: &reqwest::Client,
    req: &TtsRequest,
    creds: &TtsCredentials,
) -> Result<TtsResult, String> {
    let base = creds
        .base_url
        .as_deref()
        .unwrap_or("https://dashscope-intl.aliyuncs.com/compatible-mode/v1");
    // DashScope OpenAI-compatible TTS endpoint: /v1/audio/speech
    let url = format!(
        "{}/audio/speech",
        base.trim_end_matches('/')
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

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", creds.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Alibaba Cloud request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("Alibaba Cloud TTS error ({status}): {err}"));
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
        .map_err(|e| format!("Failed to read Alibaba Cloud response: {e}"))?;

    Ok(TtsResult {
        audio: bytes.to_vec(),
        content_type: ct,
    })
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
