//! Image-generation gateway: dispatches a unified `ImageGenerationRequest`
//! to OpenAI's `/v1/images/generations` or to fal.ai's `https://fal.run/<id>`.
//! For fal.ai, the model id IS the endpoint path.

use crate::image_types::*;
use distri_types::ProviderType;
use serde_json::{json, Value};

/// Dispatch an image-generation request to the provider's API.
pub async fn call_image_generation(
    client: &reqwest::Client,
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    match &req.provider {
        ProviderType::OpenAI => call_openai_image(client, req, creds).await,
        ProviderType::FalAi => call_fal_ai_image(client, req, creds).await,
        other => Err(format!(
            "provider '{other}' does not support image generation",
        )),
    }
}

async fn call_openai_image(
    client: &reqwest::Client,
    req: &ImageGenerationRequest,
    creds: &ImageCredentials,
) -> Result<ImageGenerationResult, String> {
    let base = creds
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1");
    let url = format!("{}/images/generations", base.trim_end_matches('/'));

    let mut body = serde_json::Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("prompt".into(), json!(req.prompt));
    if let Some(n) = req.n {
        body.insert("n".into(), json!(n));
    }
    if let Some(size) = &req.size {
        body.insert("size".into(), json!(size));
    }
    if let Some(quality) = &req.quality {
        body.insert("quality".into(), json!(quality));
    }
    if let Some(rf) = req.response_format {
        body.insert(
            "response_format".into(),
            json!(match rf {
                ImageResponseFormat::B64Json => "b64_json",
                ImageResponseFormat::Url => "url",
            }),
        );
    }
    for (k, v) in &req.extra {
        body.insert(k.clone(), v.clone());
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", creds.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("openai images request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "openai images returned {status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid openai images response: {e}"))?;
    let data = payload
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let images: Vec<ImageData> = data
        .into_iter()
        .map(|d| ImageData {
            url: d.get("url").and_then(|v| v.as_str()).map(String::from),
            b64_json: d.get("b64_json").and_then(|v| v.as_str()).map(String::from),
            revised_prompt: d
                .get("revised_prompt")
                .and_then(|v| v.as_str())
                .map(String::from),
            content_type: None,
            width: None,
            height: None,
        })
        .collect();
    Ok(ImageGenerationResult {
        provider: "openai".to_string(),
        model: req.model.clone(),
        images,
    })
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
