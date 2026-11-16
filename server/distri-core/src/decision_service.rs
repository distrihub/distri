//! Typed decisions — resolve a `"provider/model"` to a System One endpoint
//! and dispatch through [`llm_gateway::HttpDecisionProvider`].
//!
//! Resolution:
//! - A built-in decision provider (`typesafe`, `openrouter`, `aimlapi`,
//!   `workers_ai`, and the generic `decisions` provider for any System One
//!   endpoint) takes its provider settings from the secret store: the bearer
//!   token, and the base URL setting where it has one (the Workers AI account
//!   id, the generic provider's `DECISIONS_BASE_URL`). These are
//!   the same settings the Models UI edits.
//! - A `custom_*` provider resolves through
//!   [`ProviderStore::resolve_provider_endpoint`] — the same `base_url` +
//!   `{ID}_API_KEY` a custom completion provider uses — and is called at
//!   `{base_url}/systemone`. This is how a self-hosted llama-server (Kev,
//!   Laya, Clef) is reached.
//! - Anything else is [`DecisionError::UnknownProvider`].

use std::sync::Arc;

use distri_types::decision::{
    systemone_url, DecisionError, DecisionProviderKind, DecisionRequest, DecisionResponse,
    DecisionWire,
};
use distri_types::stores::{ProviderStore, SecretStore};
use llm_gateway::{DecisionEndpoint, DecisionProvider, HttpDecisionProvider};

pub struct DecisionService {
    http_client: reqwest::Client,
    secret_store: Arc<dyn SecretStore>,
    provider_store: Option<Arc<dyn ProviderStore>>,
}

impl DecisionService {
    pub fn new(
        http_client: reqwest::Client,
        secret_store: Arc<dyn SecretStore>,
        provider_store: Option<Arc<dyn ProviderStore>>,
    ) -> Self {
        Self {
            http_client,
            secret_store,
            provider_store,
        }
    }

    /// Validate, resolve the endpoint, and answer the request.
    pub async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        req.validate()?;
        let endpoint = self.resolve_endpoint(req).await?;
        HttpDecisionProvider::new(self.http_client.clone(), endpoint)
            .decide(req)
            .await
    }

    /// Where `req.model` is served, and with which credential.
    pub async fn resolve_endpoint(
        &self,
        req: &DecisionRequest,
    ) -> Result<DecisionEndpoint, DecisionError> {
        let (provider_id, model) = req.provider_and_model()?;

        if let Some(kind) = DecisionProviderKind::from_id(provider_id) {
            let api_key = match kind.api_key_required() {
                true => Some(self.required_secret(kind, kind.api_key_secret()).await?),
                false => self.secret(kind.api_key_secret()).await?,
            };
            let setting = match kind.base_url_secret() {
                Some(secret) => Some(self.required_secret(kind, secret).await?),
                None => None,
            };
            let base_url =
                kind.base_url(setting.as_deref())
                    .ok_or_else(|| DecisionError::MissingSecret {
                        provider: kind.display_name().to_string(),
                        secret: kind.base_url_secret().unwrap_or(kind.api_key_secret()),
                    })?;
            return Ok(DecisionEndpoint {
                url: kind.endpoint_url(&base_url, model),
                api_key,
                wire: kind.wire(),
            });
        }

        if !provider_id.starts_with("custom_") {
            return Err(DecisionError::UnknownProvider(provider_id.to_string()));
        }
        let store = self
            .provider_store
            .as_ref()
            .ok_or_else(|| DecisionError::UnknownProvider(provider_id.to_string()))?;
        let resolved = store
            .resolve_provider_endpoint(provider_id)
            .await
            .map_err(|e| DecisionError::ProviderLookup {
                provider: provider_id.to_string(),
                reason: e.to_string(),
            })?;
        Ok(DecisionEndpoint {
            url: systemone_url(&resolved.base_url),
            api_key: Some(resolved.api_key).filter(|k| !k.is_empty()),
            wire: DecisionWire::Systemone,
        })
    }

    /// A provider setting, `None` when unset or empty. Store failures surface.
    async fn secret(&self, key: &str) -> Result<Option<String>, DecisionError> {
        let record = self
            .secret_store
            .get(key)
            .await
            .map_err(|e| DecisionError::Store(e.to_string()))?;
        Ok(record.map(|r| r.value).filter(|v| !v.trim().is_empty()))
    }

    async fn required_secret(
        &self,
        kind: DecisionProviderKind,
        secret: &'static str,
    ) -> Result<String, DecisionError> {
        self.secret(secret)
            .await?
            .ok_or_else(|| DecisionError::MissingSecret {
                provider: kind.display_name().to_string(),
                secret,
            })
    }
}

/// A decision request as the messages its usage record is split by: the
/// questions are the instructions (`system`), the state is what the model
/// reads (`user_message`). Only for counting; the provider gets the typed
/// request.
pub fn decision_messages(
    req: &distri_types::decision::DecisionRequest,
) -> Vec<distri_types::Message> {
    let questions = serde_json::to_string(&req.questions).unwrap_or_default();
    let state = match serde_json::to_value(&req.state) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(v) => v.to_string(),
        Err(_) => String::new(),
    };
    vec![
        distri_types::Message::system(questions, None),
        distri_types::Message::user(state, None),
    ]
}

#[cfg(test)]
mod decision_parts_tests {
    use super::*;
    use distri_types::RequestPartKind;

    #[test]
    fn a_decision_splits_into_instructions_and_state() {
        let req: distri_types::decision::DecisionRequest =
            serde_json::from_value(serde_json::json!({
                "model": "custom_llama/kev",
                "state": "A customer asks why their invoice doubled this month.",
                "questions": {
                    "route": { "type": "choice", "instructions": "Who should handle this?",
                               "criteria": { "billing": null, "support": null } }
                }
            }))
            .expect("decision request");
        let parts = crate::llm_call::request_parts(&decision_messages(&req), &[], Some(173));
        let kinds: Vec<_> = parts.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&RequestPartKind::System), "{kinds:?}");
        assert!(kinds.contains(&RequestPartKind::UserMessage), "{kinds:?}");
        assert_eq!(parts.iter().map(|p| p.tokens).sum::<i64>(), 173);
    }
}
