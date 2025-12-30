use super::types::{SpeechToTextProvider, TranscribeRequest, TtsConfig, TtsProvider, TtsRequest};
use anyhow::Result;
use async_openai::{
    config::OpenAIConfig,
    types::audio::{
        AudioInput, CreateSpeechRequest, CreateTranscriptionRequest, SpeechModel,
        SpeechResponseFormat, Voice,
    },
    Client as OpenAIClient,
};
use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use reqwest;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct OpenAITtsProvider {
    client: OpenAIClient<OpenAIConfig>,
}

impl OpenAITtsProvider {
    pub fn new(api_key: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = OpenAIClient::with_config(config);

        Self { client }
    }
}

#[async_trait]
impl TtsProvider for OpenAITtsProvider {
    async fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<Bytes> {
        let voice = voice.unwrap_or("alloy");
        let speed = speed.unwrap_or(1.0);

        let request = CreateSpeechRequest {
            model: SpeechModel::Tts1,
            input: text.to_string(),
            voice: match voice {
                "alloy" => Voice::Alloy,
                "echo" => Voice::Echo,
                "fable" => Voice::Fable,
                "onyx" => Voice::Onyx,
                "nova" => Voice::Nova,
                "shimmer" => Voice::Shimmer,
                _ => Voice::Alloy,
            },
            response_format: Some(SpeechResponseFormat::Mp3),
            speed: Some(speed),
            instructions: None,
            stream_format: None,
        };

        let response = self.client.audio().speech().create(request).await?;
        Ok(response.bytes)
    }
}

#[async_trait]
impl SpeechToTextProvider for OpenAITtsProvider {
    async fn transcribe(&self, audio_data: &[u8]) -> Result<String> {
        self.transcribe_with_options(
            audio_data,
            &TranscribeRequest {
                model: Some("whisper-1".to_string()),
                language: None,
                temperature: None,
            },
        )
        .await
    }

    async fn transcribe_with_options(
        &self,
        audio_data: &[u8],
        options: &TranscribeRequest,
    ) -> Result<String> {
        use std::io::Write;

        // Create temporary file for audio
        let temp_file = std::env::temp_dir().join(format!("audio_{}.webm", uuid::Uuid::new_v4()));

        let mut file = std::fs::File::create(&temp_file)?;
        file.write_all(audio_data)?;

        let model = options
            .model
            .clone()
            .unwrap_or_else(|| "whisper-1".to_string());

        // Read file into bytes and create AudioInput
        let audio_bytes = std::fs::read(&temp_file)?;

        let request = CreateTranscriptionRequest {
            file: AudioInput::from_vec_u8("audio.webm".to_string(), audio_bytes),
            model: model,
            language: options.language.clone(),
            prompt: None,
            response_format: None,
            temperature: options.temperature,
            timestamp_granularities: None,
            ..Default::default()
        };
        let response = self.client.audio().transcription().create(request).await?;

        // Clean up temp file
        let _ = std::fs::remove_file(&temp_file);

        Ok(response.text)
    }
}

#[derive(Clone)]
pub struct GeminiTtsProvider {
    client: reqwest::Client,
    api_key: String,
}

impl GeminiTtsProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl TtsProvider for GeminiTtsProvider {
    async fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<Bytes> {
        let voice = voice.unwrap_or("en-US-Standard-A");
        let speed = speed.unwrap_or(1.0);

        #[derive(Serialize)]
        struct Input {
            text: String,
        }

        #[derive(Serialize)]
        struct Voice {
            #[serde(rename = "languageCode")]
            language_code: String,
            name: String,
        }

        #[derive(Serialize)]
        struct AudioConfig {
            #[serde(rename = "audioEncoding")]
            audio_encoding: String,
            #[serde(rename = "speakingRate")]
            speaking_rate: f32,
        }

        #[derive(Serialize)]
        struct GeminiTtsRequest {
            input: Input,
            voice: Voice,
            #[serde(rename = "audioConfig")]
            audio_config: AudioConfig,
        }

        let request = GeminiTtsRequest {
            input: Input {
                text: text.to_string(),
            },
            voice: Voice {
                language_code: "en-US".to_string(),
                name: voice.to_string(),
            },
            audio_config: AudioConfig {
                audio_encoding: "MP3".to_string(),
                speaking_rate: speed,
            },
        };

        let url = format!(
            "https://texttospeech.googleapis.com/v1/text:synthesize?key={}",
            self.api_key
        );

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Gemini TTS API error: {}", error_text));
        }

        #[derive(Deserialize)]
        struct GeminiTtsResponse {
            #[serde(rename = "audioContent")]
            audio_content: String,
        }

        let gemini_response: GeminiTtsResponse = response.json().await?;
        let audio_bytes =
            base64::engine::general_purpose::STANDARD.decode(gemini_response.audio_content)?;

        Ok(Bytes::from(audio_bytes))
    }
}

#[derive(Clone)]
pub struct TtsService {
    openai_provider: Option<OpenAITtsProvider>,
    gemini_provider: Option<GeminiTtsProvider>,
}

impl TtsService {
    pub fn new(config: TtsConfig) -> Self {
        let openai_provider = config
            .openai_api_key
            .as_ref()
            .map(|key| OpenAITtsProvider::new(key));

        let gemini_provider = config
            .gemini_api_key
            .as_ref()
            .map(|key| GeminiTtsProvider::new(key));

        Self {
            openai_provider,
            gemini_provider,
        }
    }

    pub fn new_with_providers(
        openai_provider: Option<OpenAITtsProvider>,
        gemini_provider: Option<GeminiTtsProvider>,
    ) -> Self {
        Self {
            openai_provider,
            gemini_provider,
        }
    }

    pub async fn synthesize(&self, request: &TtsRequest) -> Result<Bytes> {
        use super::types::TtsModel;

        match request.model {
            TtsModel::OpenAI => {
                if let Some(provider) = &self.openai_provider {
                    provider
                        .synthesize(&request.text, request.voice.as_deref(), request.speed)
                        .await
                } else {
                    Err(anyhow::anyhow!("OpenAI API key not configured"))
                }
            }
            TtsModel::Gemini => {
                if let Some(provider) = &self.gemini_provider {
                    provider
                        .synthesize(&request.text, request.voice.as_deref(), request.speed)
                        .await
                } else {
                    Err(anyhow::anyhow!("Gemini API key not configured"))
                }
            }
        }
    }

    pub async fn transcribe(&self, audio_data: &[u8]) -> Result<String> {
        if let Some(provider) = &self.openai_provider {
            provider.transcribe(audio_data).await
        } else {
            Err(anyhow::anyhow!(
                "OpenAI provider not configured for speech-to-text"
            ))
        }
    }

    pub async fn transcribe_with_options(
        &self,
        audio_data: &[u8],
        options: &TranscribeRequest,
    ) -> Result<String> {
        if let Some(provider) = &self.openai_provider {
            provider.transcribe_with_options(audio_data, options).await
        } else {
            Err(anyhow::anyhow!(
                "OpenAI provider not configured for speech-to-text"
            ))
        }
    }

    pub fn get_available_voices(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut voices = serde_json::Map::new();

        if self.openai_provider.is_some() {
            voices.insert(
                "openai".to_string(),
                serde_json::json!(["alloy", "echo", "fable", "onyx", "nova", "shimmer"]),
            );
        }

        if self.gemini_provider.is_some() {
            voices.insert(
                "gemini".to_string(),
                serde_json::json!([
                    "en-US-Standard-A",
                    "en-US-Standard-B",
                    "en-US-Standard-C",
                    "en-US-Standard-D",
                    "en-US-Wavenet-A",
                    "en-US-Wavenet-B"
                ]),
            );
        }

        voices
    }
}
