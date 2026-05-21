//! Image-generation request/response types — the shared shape the gateway
//! uses to dispatch to OpenAI's `/v1/images/generations` and to fal.ai.

use distri_types::ProviderType;
use serde::{Deserialize, Serialize};

/// Encoding the caller wants back. `Url` returns a hosted link, `B64Json`
/// returns the image inline as base64.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageResponseFormat {
    Url,
    B64Json,
}

impl Default for ImageResponseFormat {
    fn default() -> Self {
        Self::Url
    }
}

/// Provider-agnostic image-generation request.
#[derive(Debug, Clone)]
pub struct ImageGenerationRequest {
    pub provider: ProviderType,
    /// Provider-specific model id. For fal.ai this is the endpoint path
    /// (e.g. `fal-ai/flux/dev`).
    pub model: String,
    pub prompt: String,
    pub n: Option<u32>,
    /// Size string (`"1024x1024"`, or fal.ai's `"square_hd"` / `"landscape_4_3"`).
    pub size: Option<String>,
    /// Quality tier (`"low"` / `"medium"` / `"high"` for gpt-image-1).
    pub quality: Option<String>,
    pub response_format: Option<ImageResponseFormat>,
    /// Provider-specific pass-through fields (seed, guidance_scale, etc.).
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Credentials resolved from the secret/provider store for the call.
#[derive(Debug, Clone)]
pub struct ImageCredentials {
    pub base_url: Option<String>,
    pub api_key: String,
}

/// Provider-agnostic image-generation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationResult {
    pub provider: String,
    pub model: String,
    pub images: Vec<ImageData>,
}

/// One generated image. At least one of `url` / `b64_json` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}
