//! Typed decisions over HTTP — one client for every System One endpoint.
//!
//! TypeSafe (Jev), OpenRouter, AI/ML API and any llama-server ≥ b11361 speak
//! the same body in and out; Cloudflare Workers AI (Clef) wraps the response
//! in its v4 envelope. The wire types live in [`distri_types::decision`].

use async_trait::async_trait;
use distri_types::decision::{DecisionError, DecisionRequest, DecisionResponse, DecisionWire};
use serde::Deserialize;

/// Answers typed questions. Implemented over HTTP here; a local runtime
/// (llama.rn on device, an embedded sidecar) implements the same contract.
#[async_trait]
pub trait DecisionProvider: Send + Sync {
    async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}

/// A resolved System One endpoint: full URL, optional bearer token, and how
/// the response is wrapped.
#[derive(Debug, Clone)]
pub struct DecisionEndpoint {
    pub url: String,
    pub api_key: Option<String>,
    pub wire: DecisionWire,
}

/// [`DecisionProvider`] for any System One HTTP endpoint.
#[derive(Debug, Clone)]
pub struct HttpDecisionProvider {
    client: reqwest::Client,
    endpoint: DecisionEndpoint,
}

impl HttpDecisionProvider {
    pub fn new(client: reqwest::Client, endpoint: DecisionEndpoint) -> Self {
        Self { client, endpoint }
    }
}

/// Cloudflare API v4 envelope around a Workers AI result.
#[derive(Debug, Deserialize)]
struct CloudflareEnvelope {
    result: Option<DecisionResponse>,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<CloudflareApiError>,
}

#[derive(Debug, Deserialize)]
struct CloudflareApiError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[async_trait]
impl DecisionProvider for HttpDecisionProvider {
    async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        req.validate()?;
        let (_, provider_model) = req.provider_and_model()?;
        // The provider sees only its own model id, not Distri's prefix.
        let upstream = DecisionRequest {
            model: provider_model.to_string(),
            ..req.clone()
        };

        let mut builder = self.client.post(&self.endpoint.url).json(&upstream);
        if let Some(key) = self.endpoint.api_key.as_deref().filter(|k| !k.is_empty()) {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| DecisionError::Transport(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| DecisionError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(DecisionError::Provider {
                status: status.as_u16(),
                body,
            });
        }

        match self.endpoint.wire {
            DecisionWire::Systemone => serde_json::from_str::<DecisionResponse>(&body)
                .map_err(|e| DecisionError::Decode(format!("{e}: {}", truncate(&body)))),
            DecisionWire::CloudflareEnvelope => {
                let envelope: CloudflareEnvelope = serde_json::from_str(&body)
                    .map_err(|e| DecisionError::Decode(format!("{e}: {}", truncate(&body))))?;
                match envelope.result {
                    Some(result) if envelope.success => Ok(result),
                    _ => Err(DecisionError::Provider {
                        status: status.as_u16(),
                        body: envelope
                            .errors
                            .iter()
                            .map(|e| format!("{}: {}", e.code, e.message))
                            .collect::<Vec<_>>()
                            .join("; "),
                    }),
                }
            }
        }
    }
}

fn truncate(body: &str) -> &str {
    match body.char_indices().nth(500) {
        Some((i, _)) => &body[..i],
        None => body,
    }
}
