//! Image-generation request/response types — the shared shape the gateway
//! uses to dispatch to OpenAI's `/v1/images/generations` and to fal.ai.

use distri_types::ProviderType;
use serde::{Deserialize, Serialize};

/// Encoding the caller wants back. `Url` returns a hosted link, `B64Json`
/// returns the image inline as base64. Honored by `dall-e-*`; gpt-image-*
/// always returns base64 and rejects the field.
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

/// Output container for the generated image. gpt-image-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageOutputFormat {
    Png,
    Jpeg,
    Webp,
}

/// Quality tier. The set varies by model — `low | medium | high | auto`
/// for gpt-image-*; `standard | hd` for dall-e-3. Pass through; the
/// provider rejects mismatches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageQuality {
    Auto,
    Low,
    Medium,
    High,
    Standard,
    Hd,
}

/// Output canvas size. The set varies by model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageSize {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "256x256")]
    S256x256,
    #[serde(rename = "512x512")]
    S512x512,
    #[serde(rename = "1024x1024")]
    S1024x1024,
    #[serde(rename = "1024x1536")]
    S1024x1536,
    #[serde(rename = "1536x1024")]
    S1536x1024,
    #[serde(rename = "1024x1792")]
    S1024x1792,
    #[serde(rename = "1792x1024")]
    S1792x1024,
}

/// Content moderation profile. gpt-image-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageModeration {
    Auto,
    Low,
}

/// Background transparency. gpt-image-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageBackground {
    Auto,
    Transparent,
    Opaque,
}

/// Style hint. dall-e-3 only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageStyle {
    Vivid,
    Natural,
}

/// Provider-agnostic image-generation request.
///
/// Every parameter is strongly typed. The gateway dispatches on
/// `provider`; for OpenAI-compatible providers the fields map directly
/// onto `async_openai::types::images::CreateImageRequest` via the
/// `From` impls below. For fal.ai the relevant subset is sent.
#[derive(Debug, Clone)]
pub struct ImageGenerationRequest {
    pub provider: ProviderType,
    /// Provider-specific model id. For fal.ai this is the endpoint path
    /// (e.g. `fal-ai/flux/dev`).
    pub model: String,
    pub prompt: String,
    pub n: Option<u32>,
    pub size: Option<ImageSize>,
    pub quality: Option<ImageQuality>,
    pub response_format: Option<ImageResponseFormat>,
    pub output_format: Option<ImageOutputFormat>,
    pub output_compression: Option<u8>,
    pub moderation: Option<ImageModeration>,
    pub background: Option<ImageBackground>,
    pub style: Option<ImageStyle>,
    pub user: Option<String>,
}

// ── Conversions to async-openai's typed enums ────────────────────────
//
// These are mirror enums (one variant each) — converting at the boundary
// lets the rest of the gateway stay coupled to *our* types, not the SDK's.

use async_openai::types::images as oai;

impl ImageResponseFormat {
    pub(crate) fn to_oai(self) -> oai::ImageResponseFormat {
        match self {
            ImageResponseFormat::Url => oai::ImageResponseFormat::Url,
            ImageResponseFormat::B64Json => oai::ImageResponseFormat::B64Json,
        }
    }
}
impl ImageOutputFormat {
    pub(crate) fn to_oai(self) -> oai::ImageOutputFormat {
        match self {
            ImageOutputFormat::Png => oai::ImageOutputFormat::Png,
            ImageOutputFormat::Jpeg => oai::ImageOutputFormat::Jpeg,
            ImageOutputFormat::Webp => oai::ImageOutputFormat::Webp,
        }
    }
}
impl ImageQuality {
    pub(crate) fn to_oai(self) -> oai::ImageQuality {
        match self {
            ImageQuality::Auto => oai::ImageQuality::Auto,
            ImageQuality::Low => oai::ImageQuality::Low,
            ImageQuality::Medium => oai::ImageQuality::Medium,
            ImageQuality::High => oai::ImageQuality::High,
            ImageQuality::Standard => oai::ImageQuality::Standard,
            ImageQuality::Hd => oai::ImageQuality::HD,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ImageQuality::Auto => "auto",
            ImageQuality::Low => "low",
            ImageQuality::Medium => "medium",
            ImageQuality::High => "high",
            ImageQuality::Standard => "standard",
            ImageQuality::Hd => "hd",
        }
    }
}
impl ImageSize {
    pub(crate) fn to_oai(self) -> oai::ImageSize {
        match self {
            ImageSize::Auto => oai::ImageSize::Auto,
            ImageSize::S256x256 => oai::ImageSize::S256x256,
            ImageSize::S512x512 => oai::ImageSize::S512x512,
            ImageSize::S1024x1024 => oai::ImageSize::S1024x1024,
            ImageSize::S1024x1536 => oai::ImageSize::S1024x1536,
            ImageSize::S1536x1024 => oai::ImageSize::S1536x1024,
            ImageSize::S1024x1792 => oai::ImageSize::S1024x1792,
            ImageSize::S1792x1024 => oai::ImageSize::S1792x1024,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ImageSize::Auto => "auto",
            ImageSize::S256x256 => "256x256",
            ImageSize::S512x512 => "512x512",
            ImageSize::S1024x1024 => "1024x1024",
            ImageSize::S1024x1536 => "1024x1536",
            ImageSize::S1536x1024 => "1536x1024",
            ImageSize::S1024x1792 => "1024x1792",
            ImageSize::S1792x1024 => "1792x1024",
        }
    }
}
impl ImageModeration {
    pub(crate) fn to_oai(self) -> oai::ImageModeration {
        match self {
            ImageModeration::Auto => oai::ImageModeration::Auto,
            ImageModeration::Low => oai::ImageModeration::Low,
        }
    }
}
impl ImageBackground {
    pub(crate) fn to_oai(self) -> oai::ImageBackground {
        match self {
            ImageBackground::Auto => oai::ImageBackground::Auto,
            ImageBackground::Transparent => oai::ImageBackground::Transparent,
            ImageBackground::Opaque => oai::ImageBackground::Opaque,
        }
    }
}
impl ImageStyle {
    pub(crate) fn to_oai(self) -> oai::ImageStyle {
        match self {
            ImageStyle::Vivid => oai::ImageStyle::Vivid,
            ImageStyle::Natural => oai::ImageStyle::Natural,
        }
    }
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
