//! Shared Text-to-Speech types used by both the TTS gateway and client libraries.

use serde::{Deserialize, Serialize};

/// Supported TTS providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsProvider {
    OpenAI,
    AzureOpenai,
    Azure,
    ElevenLabs,
}

impl std::fmt::Display for TtsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TtsProvider::OpenAI => write!(f, "openai"),
            TtsProvider::AzureOpenai => write!(f, "azure_openai"),
            TtsProvider::Azure => write!(f, "azure"),
            TtsProvider::ElevenLabs => write!(f, "elevenlabs"),
        }
    }
}

/// TTS model info returned by the list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsModelInfo {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub voices: Vec<TtsVoiceInfo>,
    pub formats: Vec<String>,
}

/// Information about a TTS voice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoiceInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// TTS provider definition with required configuration keys and available models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsProviderDefinition {
    pub id: String,
    pub label: String,
    pub keys: Vec<TtsSecretKeyDefinition>,
    pub models: Vec<TtsModelInfo>,
}

/// A secret key required by a TTS provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSecretKeyDefinition {
    pub key: String,
    pub label: String,
    pub placeholder: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_true")]
    pub sensitive: bool,
}

fn default_true() -> bool {
    true
}
