//! Image-generation gateway: dispatches a unified `ImageGenerationRequest`
//! to the right provider client.
//!
//! - **OpenAI / Azure AI Foundry** — OpenAI-compatible. We use
//!   `async-openai`'s typed `Client::images().generate()` so auth, base-URL
//!   plumbing, retries, and error mapping live in one place instead of being
//!   reimplemented here. Foundry routes through this same path; the cloud
//!   caller passes `creds.base_url = https://<resource>.services.ai.azure.com/openai/v1`.
//! - **fal.ai** — not OpenAI-compatible. Custom POST to `fal.run/<endpoint>`
//!   with `Authorization: Key …`.

use crate::gateway_config::GatewayConfig;
use crate::image_types::*;
use async_openai::types::images::{
    CreateImageRequestArgs, Image as OaiImage, ImageBackground, ImageModel, ImageModeration,
    ImageOutputFormat, ImageQuality, ImageResponseFormat as OaiImageResponseFormat, ImageSize,
    ImageStyle,
};
use async_openai::Client;
use distri_types::ProviderType;
use serde_json::{json, Value};

/// Dispatch an image-generation request to the provider's API.
pub async fn call_image_generation(
    client: &reqwest::Client,
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    match &req.provider {
        // Azure AI Foundry's image API is OpenAI-compatible — same JSON shape,
        // different base URL (passed in via `creds.base_url`).
        ProviderType::OpenAI | ProviderType::AzureAiFoundry => call_openai_image(req, creds).await,
        ProviderType::FalAi => call_fal_ai_image(client, req, creds).await,
        other => Err(format!(
            "provider '{other}' does not support image generation",
        )),
    }
}

async fn call_openai_image(
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    let base_url = creds
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let config = GatewayConfig::new()
        .with_api_base(base_url)
        .with_api_key(creds.api_key.clone());
    let openai = Client::with_config(config);

    let mut builder = CreateImageRequestArgs::default();
    builder.prompt(req.prompt.clone());
    builder.model(model_id_to_oai(&req.model));
    if let Some(n) = req.n {
        builder.n(n as u8);
    }
    if let Some(size) = req.size.as_deref().and_then(parse_size) {
        builder.size(size);
    }
    if let Some(quality) = req.quality.as_deref().and_then(parse_quality) {
        builder.quality(quality);
    }
    if let Some(rf) = req.response_format.map(map_response_format) {
        builder.response_format(rf);
    }
    // gpt-image-1/2 extras — pass-through from the caller's `extra` map.
    if let Some(output_format) = req.extra.get("output_format").and_then(Value::as_str).and_then(parse_output_format) {
        builder.output_format(output_format);
    }
    if let Some(output_compression) = req.extra.get("output_compression").and_then(Value::as_u64) {
        builder.output_compression(output_compression as u8);
    }
    if let Some(moderation) = req.extra.get("moderation").and_then(Value::as_str).and_then(parse_moderation) {
        builder.moderation(moderation);
    }
    if let Some(background) = req.extra.get("background").and_then(Value::as_str).and_then(parse_background) {
        builder.background(background);
    }
    if let Some(style) = req.extra.get("style").and_then(Value::as_str).and_then(parse_style) {
        builder.style(style);
    }
    let request = builder
        .build()
        .map_err(|e| format!("invalid image-generation request: {e}"))?;

    let response = openai
        .images()
        .generate(request)
        .await
        .map_err(|e| format!("image generation failed: {e}"))?;

    let images = response
        .data
        .iter()
        .map(|img| match img.as_ref() {
            OaiImage::Url {
                url,
                revised_prompt,
            } => ImageData {
                url: Some(url.clone()),
                b64_json: None,
                revised_prompt: revised_prompt.clone(),
                content_type: None,
                width: None,
                height: None,
            },
            OaiImage::B64Json {
                b64_json,
                revised_prompt,
            } => ImageData {
                url: None,
                b64_json: Some(b64_json.as_str().to_string()),
                revised_prompt: revised_prompt.clone(),
                content_type: None,
                width: None,
                height: None,
            },
        })
        .collect();

    Ok(ImageGenerationResult {
        provider: req.provider.to_string(),
        model: req.model.clone(),
        images,
    })
}

fn model_id_to_oai(model: &str) -> ImageModel {
    match model {
        "gpt-image-1" => ImageModel::GptImage1,
        "gpt-image-1.5" => ImageModel::GptImage1dot5,
        "gpt-image-1-mini" => ImageModel::GptImage1Mini,
        "dall-e-2" => ImageModel::DallE2,
        "dall-e-3" => ImageModel::DallE3,
        // gpt-image-2 and any newer/custom id flow through the untagged variant.
        other => ImageModel::Other(other.to_string()),
    }
}

fn parse_size(s: &str) -> Option<ImageSize> {
    match s {
        "auto" => Some(ImageSize::Auto),
        "256x256" => Some(ImageSize::S256x256),
        "512x512" => Some(ImageSize::S512x512),
        "1024x1024" => Some(ImageSize::S1024x1024),
        "1792x1024" => Some(ImageSize::S1792x1024),
        "1024x1792" => Some(ImageSize::S1024x1792),
        "1536x1024" => Some(ImageSize::S1536x1024),
        "1024x1536" => Some(ImageSize::S1024x1536),
        _ => None,
    }
}

fn parse_quality(q: &str) -> Option<ImageQuality> {
    match q.to_lowercase().as_str() {
        "auto" => Some(ImageQuality::Auto),
        "low" => Some(ImageQuality::Low),
        "medium" => Some(ImageQuality::Medium),
        "high" => Some(ImageQuality::High),
        "standard" => Some(ImageQuality::Standard),
        "hd" => Some(ImageQuality::HD),
        _ => None,
    }
}

fn map_response_format(rf: ImageResponseFormat) -> OaiImageResponseFormat {
    match rf {
        ImageResponseFormat::Url => OaiImageResponseFormat::Url,
        ImageResponseFormat::B64Json => OaiImageResponseFormat::B64Json,
    }
}

fn parse_output_format(s: &str) -> Option<ImageOutputFormat> {
    match s.to_lowercase().as_str() {
        "png" => Some(ImageOutputFormat::Png),
        "jpeg" | "jpg" => Some(ImageOutputFormat::Jpeg),
        "webp" => Some(ImageOutputFormat::Webp),
        _ => None,
    }
}

fn parse_moderation(s: &str) -> Option<ImageModeration> {
    match s.to_lowercase().as_str() {
        "auto" => Some(ImageModeration::Auto),
        "low" => Some(ImageModeration::Low),
        _ => None,
    }
}

fn parse_background(s: &str) -> Option<ImageBackground> {
    match s.to_lowercase().as_str() {
        "auto" => Some(ImageBackground::Auto),
        "transparent" => Some(ImageBackground::Transparent),
        "opaque" => Some(ImageBackground::Opaque),
        _ => None,
    }
}

fn parse_style(s: &str) -> Option<ImageStyle> {
    match s.to_lowercase().as_str() {
        "vivid" => Some(ImageStyle::Vivid),
        "natural" => Some(ImageStyle::Natural),
        _ => None,
    }
}

async fn call_fal_ai_image(
    client: &reqwest::Client,
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    let base = creds.base_url.as_deref().unwrap_or("https://fal.run");
    // For fal.ai the model id IS the endpoint path (e.g. `fal-ai/flux/dev`).
    let path = req.model.trim_start_matches('/');
    let url = format!("{}/{}", base.trim_end_matches('/'), path);

    let mut body = serde_json::Map::new();
    body.insert("prompt".into(), json!(req.prompt));
    if let Some(n) = req.n {
        body.insert("num_images".into(), json!(n));
    }
    if let Some(size) = &req.size {
        body.insert("image_size".into(), json!(size));
    }
    for (k, v) in &req.extra {
        body.insert(k.clone(), v.clone());
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Key {}", creds.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("fal.ai request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "fal.ai returned {status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid fal.ai response: {e}"))?;
    let imgs = payload
        .get("images")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let images: Vec<ImageData> = imgs
        .into_iter()
        .map(|d| ImageData {
            url: d.get("url").and_then(|v| v.as_str()).map(String::from),
            b64_json: None,
            content_type: d
                .get("content_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            width: d.get("width").and_then(|v| v.as_u64()).map(|w| w as u32),
            height: d.get("height").and_then(|v| v.as_u64()).map(|h| h as u32),
            revised_prompt: None,
        })
        .collect();
    Ok(ImageGenerationResult {
        provider: "fal.ai".to_string(),
        model: req.model.clone(),
        images,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live test: gpt-image-1 returns a non-empty image. Requires `OPENAI_API_KEY`.
    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn openai_gpt_image_1_returns_image() {
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        assert!(!api_key.is_empty(), "OPENAI_API_KEY must be set");
        let creds = ImageCredentials {
            base_url: None,
            api_key,
        };
        let req = ImageGenerationRequest {
            provider: ProviderType::OpenAI,
            model: "gpt-image-1".to_string(),
            prompt: "a single small red square on a white background".to_string(),
            n: Some(1),
            size: Some("1024x1024".to_string()),
            quality: Some("low".to_string()),
            response_format: None,
            extra: serde_json::Map::new(),
        };
        let result = call_image_generation(&reqwest::Client::new(), &req, &creds)
            .await
            .expect("OpenAI image generation");
        assert_eq!(result.images.len(), 1);
        let img = &result.images[0];
        assert!(
            img.url.is_some() || img.b64_json.is_some(),
            "OpenAI image must carry either a url or b64_json"
        );
    }

    /// Live test: fal.ai flux schnell returns a non-empty image. Requires `FAL_KEY`.
    #[tokio::test]
    #[ignore = "requires FAL_KEY"]
    async fn fal_ai_flux_schnell_returns_image() {
        let api_key = std::env::var("FAL_KEY").unwrap_or_default();
        assert!(!api_key.is_empty(), "FAL_KEY must be set");
        let creds = ImageCredentials {
            base_url: None,
            api_key,
        };
        let req = ImageGenerationRequest {
            provider: ProviderType::FalAi,
            model: "fal-ai/flux/schnell".to_string(),
            prompt: "a single small red square on a white background".to_string(),
            n: Some(1),
            size: None,
            quality: None,
            response_format: None,
            extra: serde_json::Map::new(),
        };
        let result = call_image_generation(&reqwest::Client::new(), &req, &creds)
            .await
            .expect("fal.ai image generation");
        assert!(!result.images.is_empty());
        assert!(result.images[0].url.is_some());
    }
}
