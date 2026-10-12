//! `DecisionService` endpoint resolution + dispatch, against an HTTP mock.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use distri_types::decision::{DecisionError, DecisionRequest, DecisionWire};
use distri_types::stores::{
    NewSecret, ProviderStore, ResolvedProviderEndpoint, SecretRecord, SecretStore,
    UpsertProviderRequest, UpsertProviderResponse,
};
use wiremock::matchers::{bearer_token, body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::decision_service::DecisionService;

struct Secrets(HashMap<String, String>);

#[async_trait::async_trait]
impl SecretStore for Secrets {
    async fn list(&self) -> anyhow::Result<Vec<SecretRecord>> {
        unimplemented!()
    }
    async fn get(&self, key: &str) -> anyhow::Result<Option<SecretRecord>> {
        Ok(self.0.get(key).map(|value| SecretRecord {
            id: key.to_string(),
            key: key.to_string(),
            value: value.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }
    async fn create(&self, _secret: NewSecret) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn update(&self, _key: &str, _value: &str) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

struct FailingSecrets;

#[async_trait::async_trait]
impl SecretStore for FailingSecrets {
    async fn list(&self) -> anyhow::Result<Vec<SecretRecord>> {
        unimplemented!()
    }
    async fn get(&self, _key: &str) -> anyhow::Result<Option<SecretRecord>> {
        anyhow::bail!("connection refused")
    }
    async fn create(&self, _secret: NewSecret) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn update(&self, _key: &str, _value: &str) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

/// Custom providers keyed by id → (base_url, api_key).
struct Providers(HashMap<String, (String, String)>);

#[async_trait::async_trait]
impl ProviderStore for Providers {
    async fn upsert_provider(
        &self,
        _req: UpsertProviderRequest,
    ) -> anyhow::Result<UpsertProviderResponse> {
        unimplemented!()
    }
    async fn delete_provider(&self, _provider_id: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn get_default_model(&self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn resolve_provider_endpoint(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ResolvedProviderEndpoint> {
        let (base_url, api_key) = self
            .0
            .get(provider_id)
            .ok_or_else(|| anyhow::anyhow!("unknown provider '{provider_id}'"))?;
        Ok(ResolvedProviderEndpoint {
            base_url: base_url.clone(),
            api_key: api_key.clone(),
        })
    }
}

fn request(model: &str) -> DecisionRequest {
    serde_json::from_value(serde_json::json!({
        "model": model,
        "state": "Checkout has been failing for every customer for the last hour.",
        "questions": {
            "team": {"type": "choice", "instructions": "Which team?",
                     "criteria": {"billing": null, "technical": "Outages and errors"}},
            "urgent": {"type": "noul", "instructions": "Is this urgent?"}
        }
    }))
    .unwrap()
}

const ANSWER: &str = r#"{
    "model": "kev",
    "answers": {
        "team": {"type": "choice", "choice": "technical",
                 "probabilities": {"billing": 0.1, "technical": 0.9}, "confidence": 0.8},
        "urgent": {"type": "noul", "noul": 0.97}
    },
    "usage": {"input_tokens": 52, "output_tokens": 0}
}"#;

fn secrets(pairs: &[(&str, &str)]) -> Arc<dyn SecretStore> {
    Arc::new(Secrets(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    ))
}

#[tokio::test]
async fn builtin_provider_uses_its_secret_and_hosted_url() {
    let svc = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[("TYPESAFE_API_KEY", "ts-key")]),
        None,
    );
    let ep = svc
        .resolve_endpoint(&request("typesafe/jev-latest"))
        .await
        .unwrap();
    assert_eq!(ep.url, "https://api.typesafe.ai/v1/systemone");
    assert_eq!(ep.api_key.as_deref(), Some("ts-key"));
    assert_eq!(ep.wire, DecisionWire::Systemone);
}

#[tokio::test]
async fn workers_ai_needs_token_and_account() {
    let only_token = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[("CLOUDFLARE_API_TOKEN", "cf")]),
        None,
    );
    match only_token
        .resolve_endpoint(&request("workers_ai/clef-flash"))
        .await
    {
        Err(DecisionError::MissingSecret { secret, .. }) => {
            assert_eq!(secret, "CLOUDFLARE_ACCOUNT_ID")
        }
        other => panic!("expected MissingSecret, got {other:?}"),
    }

    let both = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[
            ("CLOUDFLARE_API_TOKEN", "cf"),
            ("CLOUDFLARE_ACCOUNT_ID", "acc9"),
        ]),
        None,
    );
    let ep = both
        .resolve_endpoint(&request("workers_ai/clef-flash"))
        .await
        .unwrap();
    assert_eq!(
        ep.url,
        "https://api.cloudflare.com/client/v4/accounts/acc9/ai/run/@cf/cloudflare/clef-flash"
    );
    assert_eq!(ep.wire, DecisionWire::CloudflareEnvelope);
}

#[tokio::test]
async fn missing_and_empty_keys_are_reported_by_name() {
    for store in [secrets(&[]), secrets(&[("OPENROUTER_API_KEY", "")])] {
        let svc = DecisionService::new(reqwest::Client::new(), store, None);
        let err = svc
            .decide(&request("openrouter/typesafe/jev"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("OPENROUTER_API_KEY"), "{err}");
        assert_eq!(distri_types::ApiError::from(err).status(), 400);
    }
}

#[tokio::test]
async fn secret_store_failures_surface() {
    let svc = DecisionService::new(reqwest::Client::new(), Arc::new(FailingSecrets), None);
    let err = svc
        .decide(&request("typesafe/jev-latest"))
        .await
        .unwrap_err();
    assert!(matches!(&err, DecisionError::Store(msg) if msg.contains("connection refused")));
    assert_eq!(distri_types::ApiError::from(err).status(), 500);
}

#[tokio::test]
async fn non_decision_providers_are_rejected() {
    let svc = DecisionService::new(reqwest::Client::new(), secrets(&[]), None);
    for model in ["openai/gpt-5", "anthropic/claude", "typo/jev"] {
        assert!(
            matches!(
                svc.decide(&request(model)).await,
                Err(DecisionError::UnknownProvider(_))
            ),
            "{model}"
        );
    }
}

#[tokio::test]
async fn decisions_provider_provider_reads_base_url_and_optional_key_from_settings() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(bearer_token("dev-key"))
        .and(body_partial_json(serde_json::json!({"model": "kev"})))
        .respond_with(ResponseTemplate::new(200).set_body_string(ANSWER))
        .expect(1)
        .mount(&server)
        .await;

    let svc = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[
            ("DECISIONS_BASE_URL", &format!("{}/v1/", server.uri())),
            ("DECISIONS_API_KEY", "dev-key"),
        ]),
        None,
    );
    let res = svc.decide(&request("decisions/kev")).await.unwrap();
    assert_eq!(res.answers.len(), 2);

    // No key: allowed (a llama-server may run without --api-key), no bearer.
    let no_key = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[("DECISIONS_BASE_URL", "http://127.0.0.1:8090/v1")]),
        None,
    );
    let ep = no_key
        .resolve_endpoint(&request("decisions/kev"))
        .await
        .unwrap();
    assert_eq!(ep.url, "http://127.0.0.1:8090/v1/systemone");
    assert_eq!(ep.api_key, None);
}

#[tokio::test]
async fn decisions_provider_without_base_url_names_the_setting() {
    let svc = DecisionService::new(reqwest::Client::new(), secrets(&[]), None);
    let err = svc.decide(&request("decisions/kev")).await.unwrap_err();
    assert!(err.to_string().contains("DECISIONS_BASE_URL"), "{err}");
    assert_eq!(distri_types::ApiError::from(err).status(), 400);
}

#[tokio::test]
async fn custom_provider_is_called_at_systemone_under_its_base_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(bearer_token("local-key"))
        .and(body_partial_json(serde_json::json!({"model": "kev-0.8b"})))
        .respond_with(ResponseTemplate::new(200).set_body_string(ANSWER))
        .expect(1)
        .mount(&server)
        .await;

    let providers = Providers(HashMap::from([(
        "custom_llama".to_string(),
        (format!("{}/v1", server.uri()), "local-key".to_string()),
    )]));
    let svc = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[]),
        Some(Arc::new(providers)),
    );
    let res = svc.decide(&request("custom_llama/kev-0.8b")).await.unwrap();
    assert_eq!(res.usage.input_tokens, 52);
    assert_eq!(res.answers.len(), 2);
}

#[tokio::test]
async fn custom_provider_without_key_sends_no_bearer() {
    let providers = Providers(HashMap::from([(
        "custom_llama".to_string(),
        ("http://127.0.0.1:8080/v1".to_string(), String::new()),
    )]));
    let svc = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[]),
        Some(Arc::new(providers)),
    );
    let ep = svc
        .resolve_endpoint(&request("custom_llama/kev"))
        .await
        .unwrap();
    assert_eq!(ep.url, "http://127.0.0.1:8080/v1/systemone");
    assert_eq!(ep.api_key, None);
}

#[tokio::test]
async fn unconfigured_custom_provider_is_a_lookup_error() {
    let svc = DecisionService::new(
        reqwest::Client::new(),
        secrets(&[]),
        Some(Arc::new(Providers(HashMap::new()))),
    );
    let err = svc
        .decide(&request("custom_missing/kev"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DecisionError::ProviderLookup { provider, .. } if provider == "custom_missing")
    );
    assert_eq!(distri_types::ApiError::from(err).status(), 400);
}
