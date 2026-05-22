//! Image-generation gateway: dispatches a typed `ImageGenerationRequest`
//! to the right provider client.
//!
//! - **OpenAI / Azure AI Foundry** — OpenAI-compatible. We use
//!   `async-openai`'s typed `Client::images().generate()` so auth,
//!   base-URL plumbing, and error mapping live in one place. Foundry
//!   routes through this same path; the cloud caller supplies the
//!   right `creds.base_url`. Per-field typed enums map onto async-openai
//!   variants via `ImageGenerationRequest`'s `to_oai()` conversions —
//!   no JSON map lookups.
//! - **fal.ai** — not OpenAI-compatible. Custom POST to `fal.run/<endpoint>`
//!   with `Authorization: Key …`.

use crate::gateway_config::GatewayConfig;
use crate::image_types::*;
use async_openai::types::images::{
    CreateImageRequestArgs, Image as OaiImage, ImageModel,
};
use async_openai::Client;
use distri_types::ProviderType;
use serde_json::{json, Value};

pub async fn call_image_generation(
    client: &reqwest::Client,
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    match &req.provider {
        // Azure AI Foundry's image API is OpenAI-compatible — same JSON
        // shape, different base URL (passed via `creds.base_url`).
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

    let oai_model = model_id_to_oai(&req.model);
    let is_gpt_image = is_gpt_image_family(&oai_model);

    let mut builder = CreateImageRequestArgs::default();
    builder.prompt(req.prompt.clone());
    builder.model(oai_model);
    if let Some(n) = req.n {
        builder.n(n as u8);
    }
    if let Some(size) = req.size {
        builder.size(size.to_oai());
    }
    if let Some(quality) = req.quality {
        builder.quality(quality.to_oai());
    }
    // gpt-image-* always returns base64 and the OpenAI API rejects an
    // explicit `response_format` on those models. dall-e-2/3 still honor
    // it. Decision is on the typed `ImageModel` variant, not a string.
    if !is_gpt_image {
        if let Some(rf) = req.response_format {
            builder.response_format(rf.to_oai());
        }
    }
    // gpt-image-only fields. async-openai's builder ignores them as long
    // as we don't call the setter, so simply gate by `is_gpt_image`.
    if is_gpt_image {
        if let Some(of) = req.output_format {
            builder.output_format(of.to_oai());
        }
        if let Some(oc) = req.output_compression {
            builder.output_compression(oc);
        }
        if let Some(m) = req.moderation {
            builder.moderation(m.to_oai());
        }
        if let Some(b) = req.background {
            builder.background(b.to_oai());
        }
    }
    // dall-e-3 only.
    if matches!(oai_model_ref(&req.model), ImageModel::DallE3) {
        if let Some(s) = req.style {
            builder.style(s.to_oai());
        }
    }
    if let Some(u) = &req.user {
        builder.user(u.clone());
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
        // gpt-image-2 and any custom id flow through the untagged variant.
        other => ImageModel::Other(other.to_string()),
    }
}

/// Cheap re-parse used only for the dall-e-3-style guard, so we don't
/// need to clone the enum value through the builder chain.
fn oai_model_ref(model: &str) -> ImageModel {
    model_id_to_oai(model)
}

/// True for any gpt-image model — including future versions whose ids
/// fall into `ImageModel::Other(...)` (e.g. `gpt-image-2`).
fn is_gpt_image_family(model: &ImageModel) -> bool {
    matches!(
        model,
        ImageModel::GptImage1 | ImageModel::GptImage1dot5 | ImageModel::GptImage1Mini,
    ) || matches!(model, ImageModel::Other(s) if s.starts_with("gpt-image"))
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
    if let Some(size) = req.size {
        body.insert("image_size".into(), json!(size.as_str()));
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
            size: Some(ImageSize::S1024x1024),
            quality: Some(ImageQuality::Low),
            response_format: None,
            output_format: None,
            output_compression: None,
            moderation: None,
            background: None,
            style: None,
            user: None,
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
            output_format: None,
            output_compression: None,
            moderation: None,
            background: None,
            style: None,
            user: None,
        };
        let result = call_image_generation(&reqwest::Client::new(), &req, &creds)
            .await
            .expect("fal.ai image generation");
        assert!(!result.images.is_empty());
        assert!(result.images[0].url.is_some());
    }
}
