use serde::{Deserialize, Serialize};

// Re-export shared types from distri-types
pub use distri_types::{
    Model, ModelCapability, ModelPricing, ModelProviderDefinition, ProviderKeyDefinition,
    ProviderType, TtsVoiceInfo,
};

/// Audio output format.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Mp3,
    Wav,
    Opus,
    Aac,
    Flac,
    Pcm,
}

impl AudioFormat {
    pub fn content_type(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Opus => "audio/opus",
            AudioFormat::Aac => "audio/aac",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Pcm => "audio/pcm",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Wav => "wav",
            AudioFormat::Opus => "opus",
            AudioFormat::Aac => "aac",
            AudioFormat::Flac => "flac",
            AudioFormat::Pcm => "pcm",
        }
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        AudioFormat::Mp3
    }
}

/// Request to generate speech.
#[derive(Debug, Deserialize)]
pub struct TtsRequest {
    pub input: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_provider")]
    pub provider: ProviderType,
    #[serde(default)]
    pub response_format: AudioFormat,
    pub speed: Option<f32>,
    pub instructions: Option<String>,
    // Azure OpenAI
    pub azure_deployment: Option<String>,
    // Azure Cognitive Services
    pub azure_region: Option<String>,
    // ElevenLabs
    pub voice_id: Option<String>,
    pub elevenlabs_model_id: Option<String>,
}

fn default_model() -> String {
    "tts-1".to_string()
}
fn default_voice() -> String {
    "alloy".to_string()
}
fn default_provider() -> ProviderType {
    ProviderType::OpenAI
}

/// Resolved credentials for a TTS provider.
#[derive(Debug, Clone)]
pub struct TtsCredentials {
    pub api_key: String,
    pub base_url: Option<String>,
    pub region: Option<String>,
}

/// Result from a TTS call: audio bytes + content type.
pub struct TtsResult {
    pub audio: Vec<u8>,
    pub content_type: String,
}
