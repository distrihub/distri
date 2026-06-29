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
    #[serde(rename = "fal_ai")]
    FalAi,
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
            Self::FalAi => "fal_ai",
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
            Self::FalAi => "fal.ai",
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
            "fal_ai" => Self::FalAi,
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
    Image,
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
    /// Image generation pricing — per image (USD), with optional per-quality
    /// overrides keyed by quality tier name (`"low"` / `"medium"` / `"high"`
    /// for gpt-image-1, `"standard"` / `"hd"` for dall-e-3, etc.).
    Image {
        per_image: f64,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        per_quality: std::collections::BTreeMap<String, f64>,
    },
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
    /// Per-provider override of how `/v1/providers/test` validates the API
    /// key. When omitted, the test endpoint probes `GET {base_url}/models`.
    /// fal.ai sets this because it has no `/models` listing endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<ProviderTestConfig>,
}

/// Per-provider override of the `/v1/providers/test` probe.
///
/// Default behavior (when omitted): `GET {base_url}/models` with both
/// `Authorization: Bearer <key>` and `api-key: <key>` headers, parsing
/// `{data: [{id}]}`.
///
/// Set this when a provider has no `/models` listing endpoint (fal.ai).
/// The probe sends the configured request and treats any response status
/// outside the configured fail set (default: 401/403) as proof the auth
/// header was accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTestConfig {
    /// Full URL, or a template containing `{base_url}`.
    pub url: String,
    /// HTTP method. Default `GET`.
    #[serde(default = "default_test_method")]
    pub method: String,
    /// Auth header style: `bearer` (default), `key` (fal.ai), or `api_key`.
    #[serde(default = "default_test_auth")]
    pub auth: String,
    /// Optional JSON body (POST/PUT). For fal.ai we send a body that fails
    /// validation (`{}`) so we never pay for a generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// HTTP status codes that count as success. When empty (default), any
    /// status other than 401/403 passes — the auth header reached the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept_status: Vec<u16>,
}

fn default_test_method() -> String {
    "GET".to_string()
}

fn default_test_auth() -> String {
    "bearer".to_string()
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
