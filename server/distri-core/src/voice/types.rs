use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsModel {
    OpenAI,
    Gemini,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub model: TtsModel,
    pub voice: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeRequest {
    pub model: Option<String>,
    pub language: Option<String>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct TtsConfig {
    pub openai_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
}

impl TtsConfig {
    pub fn from_env() -> Self {
        Self {
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            gemini_api_key: std::env::var("GEMINI_API_KEY").ok(),
        }
    }
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<Bytes>;
}

#[async_trait]
pub trait SpeechToTextProvider: Send + Sync {
    async fn transcribe(&self, audio_data: &[u8]) -> Result<String>;
    async fn transcribe_with_options(
        &self,
        audio_data: &[u8],
        options: &TranscribeRequest,
    ) -> Result<String>;
}

// Streaming voice event types for realtime communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VoiceStreamEvent {
    #[serde(rename = "audio_chunk")]
    AudioChunk {
        data: Vec<u8>,
        sample_rate: u32,
        channels: u16,
        format: AudioFormat,
    },
    #[serde(rename = "text_chunk")]
    TextChunk { text: String, is_final: bool },
    #[serde(rename = "speech_started")]
    SpeechStarted,
    #[serde(rename = "speech_ended")]
    SpeechEnded,
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Wav,
    Mp3,
    Webm,
    Ogg,
}

// Real-time streaming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    pub model: String,
    pub voice: Option<String>,
    pub language: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4.1-realtime-preview-2024-12-17".to_string(),
            voice: Some("alloy".to_string()),
            language: Some("en".to_string()),
            sample_rate: 24000,
            channels: 1,
            buffer_size: 4096,
        }
    }
}
