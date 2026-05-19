//! Core model, provider, and audio types used across the entire system.

use serde::{Deserialize, Serialize};

// ── Provider identity ───────────────────────────────────────────────────

/// Known provider types. Used for identity and dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    #[serde(rename = "openai")]
    OpenAI,
    Anthropic,
    Azure,
    Gemini,
    AzureAiFoundry,
    AwsBedrock,
    GoogleVertex,
    AlibabaCloud,
    #[serde(rename = "elevenlabs")]
    ElevenLabs,
    /// User-defined provider (LangDB-compatible / OpenAI-compatible)
    #[serde(untagged)]
    Custom(String),
}

impl ProviderType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Azure => "azure",
            Self::Gemini => "gemini",
            Self::AzureAiFoundry => "azure_ai_foundry",
            Self::AwsBedrock => "aws_bedrock",
            Self::GoogleVertex => "google_vertex",
            Self::AlibabaCloud => "alibaba_cloud",
            Self::ElevenLabs => "elevenlabs",
            Self::Custom(id) => id.as_str(),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Azure => "Azure",
            Self::Gemini => "Google Gemini",
            Self::AzureAiFoundry => "Azure AI Foundry",
            Self::AwsBedrock => "AWS Bedrock",
            Self::GoogleVertex => "Google Vertex AI",
            Self::AlibabaCloud => "Alibaba Cloud",
            Self::ElevenLabs => "ElevenLabs",
            Self::Custom(id) => id.as_str(),
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "openai" => Self::OpenAI,
            "anthropic" => Self::Anthropic,
            "azure" | "azure_openai" | "azure_speech" => Self::Azure,
            "gemini" => Self::Gemini,
            "azure_ai_foundry" => Self::AzureAiFoundry,
            "aws_bedrock" => Self::AwsBedrock,
            "google_vertex" => Self::GoogleVertex,
            "alibaba_cloud" => Self::AlibabaCloud,
            "elevenlabs" => Self::ElevenLabs,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Model types ─────────────────────────────────────────────────────────

/// What a model can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    Completion,
    Tts,
    Stt,
}

/// Pricing varies by capability type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelPricing {
    /// Completion model pricing — per 1M tokens (USD).
    Completion {
        input: f64,
        output: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_input: Option<f64>,
    },
    /// TTS pricing — per 1M characters (USD).
    Tts { per_1m_chars: f64 },
    /// STT pricing — per minute of audio (USD).
    Stt { per_minute: f64 },
}

/// A model with its capability, pricing, and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    /// Human-readable name. Optional in config sources — when omitted it is
    /// backfilled from `id` by `register_provider_extensions`.
    #[serde(default)]
    pub name: String,
    pub capability: ModelCapability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voices: Vec<TtsVoiceInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<String>,
}

/// A model with denormalized provider info — returned by GET /v1/models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelWithProvider {
    #[serde(flatten)]
    pub model: Model,
    pub provider_id: String,
    pub provider_label: String,
    pub configured: bool,
}

// ── Provider definition ─────────────────────────────────────────────────

/// Secret key definition for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderKeyDefinition {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_true")]
    pub sensitive: bool,
    /// When set, the UI renders this field as a resource segment embedded in
    /// the URL template (`{}` marks the editable segment), showing the full
    /// endpoint read-only around it. Azure AI Foundry uses this: the user
    /// edits only the resource name and that is all we store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
}

/// A provider definition with its keys and available models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProviderDefinition {
    pub id: String,
    pub label: String,
    pub keys: Vec<ProviderKeyDefinition>,
    pub models: Vec<Model>,
    #[serde(default)]
    pub is_custom: bool,
}

// ── TTS voice info ──────────────────────────────────────────────────────

/// Information about a TTS voice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsVoiceInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

fn default_true() -> bool {
    true
}
