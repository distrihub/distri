//! Typed decisions — the TypeSafe "System One" wire format.
//!
//! A decision model answers typed questions about a `state` and returns a
//! probability distribution per question instead of generated text. The wire
//! shape is TypeSafe's `POST /v1/systemone` (<https://docs.typesafe.ai/api>),
//! which is also served by OpenRouter, AI/ML API, Cloudflare Workers AI (Clef)
//! and stock llama.cpp `llama-server` ≥ b11361 (Kev, Laya, Clef, …).
//!
//! Option order is meaningful (score levels, legend order, answer order), so
//! every keyed collection here is an [`IndexMap`].

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Most options a `choice` question may carry (TypeSafe API limit).
pub const MAX_CHOICE_OPTIONS: usize = 255;
/// Fewest levels a `score` question may carry.
pub const MIN_SCORE_LEVELS: usize = 2;
/// Most levels a `score` question may carry.
pub const MAX_SCORE_LEVELS: usize = 10;

/// `POST /v1/decisions` request body — identical to TypeSafe's
/// `/v1/systemone` body, with `model` in Distri's `"provider/model"` form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct DecisionRequest {
    /// `"provider/model"`, e.g. `"typesafe/jev-latest"` or `"my_llama/kev"`.
    /// Providers receive only the part after the first `/`.
    pub model: String,
    pub state: DecisionContent,
    pub questions: IndexMap<String, DecisionQuestion>,
}

/// User content the model reads: plain text, or structured data that the
/// model renders as key/value lines. Distri never inspects inside it — it is
/// a message body, not a configuration shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(untagged)]
pub enum DecisionContent {
    Text(String),
    Structured(serde_json::Value),
}

impl From<&str> for DecisionContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<String> for DecisionContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

/// One typed question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionQuestion {
    /// Pick one option. `criteria` maps option key → optional description.
    Choice {
        instructions: DecisionContent,
        criteria: IndexMap<String, Option<String>>,
    },
    /// Place on an ordered scale. `criteria` lists level descriptions,
    /// lowest first.
    Score {
        instructions: DecisionContent,
        criteria: Vec<String>,
    },
    /// Yes/no. The answer is the probability of yes.
    Noul {
        instructions: DecisionContent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

/// Optional descriptions of what "yes" and "no" mean for a noul question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct NoulCriteria {
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub yes: Option<String>,
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub no: Option<String>,
}

/// `POST /v1/decisions` response body — TypeSafe's `/v1/systemone` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct DecisionResponse {
    /// The model that answered, as the provider reports it.
    pub model: String,
    pub answers: IndexMap<String, DecisionAnswer>,
    #[serde(default)]
    pub usage: DecisionUsage,
}

/// One answer, keyed in [`DecisionResponse::answers`] by its question id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAnswer {
    Choice {
        choice: String,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    },
    Score {
        /// Probability-weighted level index (0 = lowest level).
        score: f64,
        /// Level index (as a string) → level description.
        #[serde(default)]
        legend: IndexMap<String, String>,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    },
    Noul {
        /// Probability of yes, 0..=1.
        noul: f64,
    },
}

/// Token usage. Decision models generate nothing, so `output_tokens` is 0
/// for every provider seen so far.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema,
)]
pub struct DecisionUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

impl DecisionRequest {
    /// `(provider_id, provider_model)` split of [`Self::model`].
    pub fn provider_and_model(&self) -> Result<(&str, &str), DecisionError> {
        self.model
            .split_once('/')
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .ok_or_else(|| DecisionError::InvalidModel(self.model.clone()))
    }

    /// Check the TypeSafe API limits before dispatch, so every provider —
    /// hosted or local — rejects a malformed request with the same error.
    pub fn validate(&self) -> Result<(), DecisionError> {
        self.provider_and_model()?;
        if self.questions.is_empty() {
            return Err(DecisionError::InvalidRequest(
                "questions must contain at least one question".into(),
            ));
        }
        for (id, question) in &self.questions {
            if id.trim().is_empty() {
                return Err(DecisionError::InvalidRequest(
                    "question ids cannot be empty".into(),
                ));
            }
            match question {
                DecisionQuestion::Choice { criteria, .. } => {
                    if criteria.is_empty() || criteria.len() > MAX_CHOICE_OPTIONS {
                        return Err(DecisionError::InvalidRequest(format!(
                            "question '{id}': choice needs 1 to {MAX_CHOICE_OPTIONS} options, got {}",
                            criteria.len()
                        )));
                    }
                }
                DecisionQuestion::Score { criteria, .. } => {
                    if !(MIN_SCORE_LEVELS..=MAX_SCORE_LEVELS).contains(&criteria.len()) {
                        return Err(DecisionError::InvalidRequest(format!(
                            "question '{id}': score needs {MIN_SCORE_LEVELS} to {MAX_SCORE_LEVELS} levels, got {}",
                            criteria.len()
                        )));
                    }
                }
                DecisionQuestion::Noul { .. } => {}
            }
        }
        Ok(())
    }
}

/// Built-in decision providers. They all speak one wire format, the TypeSafe
/// System One API, at different addresses:
///
/// - [`Self::SystemOne`] is the generic **`decisions`** provider: any System One
///   endpoint at a configured `DECISIONS_BASE_URL` (llama.cpp `llama-server`
///   >= b11361 serving Kev/Laya/Clef, TypeSafe, OpenRouter, …), with an optional
///   `DECISIONS_API_KEY`. Its settings are edited like any other provider's.
/// - The rest are presets for hosted endpoints whose address is known, or
///   whose path/envelope differs (AI/ML API, Workers AI).
///
/// A `custom_*` provider id is resolved through the workspace's custom
/// providers instead, for a second System One endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionProviderKind {
    /// TypeSafe — Jev.
    Typesafe,
    /// OpenRouter's System One endpoint.
    Openrouter,
    /// AI/ML API's `/v1/decisions`.
    Aimlapi,
    /// Cloudflare Workers AI — Clef / Clef-flash.
    WorkersAi,
    /// A self-hosted llama.cpp `llama-server` serving a decision model
    /// (Kev, Laya, Clef, …) at `DECISIONS_BASE_URL`.
    SystemOne,
}

/// How a provider's HTTP response wraps the [`DecisionResponse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionWire {
    /// The body is the response.
    Systemone,
    /// Cloudflare v4 envelope: `{ "result": <response>, "success", "errors" }`.
    CloudflareEnvelope,
}

impl DecisionProviderKind {
    pub const ALL: [Self; 5] = [
        Self::Typesafe,
        Self::Openrouter,
        Self::Aimlapi,
        Self::WorkersAi,
        Self::SystemOne,
    ];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Typesafe => "typesafe",
            Self::Openrouter => "openrouter",
            Self::Aimlapi => "aimlapi",
            Self::WorkersAi => "workers_ai",
            Self::SystemOne => "decisions",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Typesafe => "TypeSafe",
            Self::Openrouter => "OpenRouter",
            Self::Aimlapi => "AI/ML API",
            Self::WorkersAi => "Cloudflare Workers AI",
            Self::SystemOne => "Decisions",
        }
    }

    /// Provider setting holding the bearer token.
    pub fn api_key_secret(&self) -> &'static str {
        match self {
            Self::Typesafe => "TYPESAFE_API_KEY",
            Self::Openrouter => "OPENROUTER_API_KEY",
            Self::Aimlapi => "AIMLAPI_API_KEY",
            Self::WorkersAi => "CLOUDFLARE_API_TOKEN",
            Self::SystemOne => "DECISIONS_API_KEY",
        }
    }

    /// Whether a call without the key is a configuration error. A generic
    /// System One endpoint (e.g. a llama-server without `--api-key`) may be open.
    pub fn api_key_required(&self) -> bool {
        !matches!(self, Self::SystemOne)
    }

    /// Provider setting the base URL is built from, if any: the Workers AI
    /// account id, or the generic provider's `DECISIONS_BASE_URL`.
    pub fn base_url_secret(&self) -> Option<&'static str> {
        match self {
            Self::WorkersAi => Some("CLOUDFLARE_ACCOUNT_ID"),
            Self::SystemOne => Some("DECISIONS_BASE_URL"),
            _ => None,
        }
    }

    /// Base URL; `setting` is the value of [`Self::base_url_secret`]. `None`
    /// when the provider needs a setting that is missing.
    pub fn base_url(&self, setting: Option<&str>) -> Option<String> {
        let setting = setting.map(str::trim).filter(|s| !s.is_empty());
        match self {
            Self::Typesafe => Some("https://api.typesafe.ai/v1".into()),
            Self::Openrouter => Some("https://openrouter.ai/api/v1".into()),
            Self::Aimlapi => Some("https://api.aimlapi.com/v1".into()),
            Self::WorkersAi => setting.map(|account| {
                format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai")
            }),
            Self::SystemOne => setting.map(|url| url.trim_end_matches('/').to_string()),
        }
    }

    /// Endpoint for `model` under `base_url`.
    pub fn endpoint_url(&self, base_url: &str, model: &str) -> String {
        let base = base_url.trim_end_matches('/');
        match self {
            Self::Aimlapi => format!("{base}/decisions"),
            Self::WorkersAi => format!("{base}/run/@cf/cloudflare/{model}"),
            Self::Typesafe | Self::Openrouter | Self::SystemOne => systemone_url(base),
        }
    }

    pub fn wire(&self) -> DecisionWire {
        match self {
            Self::WorkersAi => DecisionWire::CloudflareEnvelope,
            _ => DecisionWire::Systemone,
        }
    }
}

/// System One endpoint under a custom provider's `base_url`. Custom providers
/// store an OpenAI-style base (`http://host:8080/v1`), so the path is
/// appended, not replaced.
pub fn systemone_url(base_url: &str) -> String {
    format!("{}/systemone", base_url.trim_end_matches('/'))
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("model must be in 'provider/model' form (e.g. 'typesafe/jev-latest'), got '{0}'")]
    InvalidModel(String),
    #[error("invalid decision request: {0}")]
    InvalidRequest(String),
    #[error(
        "unknown decision provider '{0}': use typesafe, openrouter, aimlapi, workers_ai, \
         or a custom_* provider whose base_url serves /systemone"
    )]
    UnknownProvider(String),
    #[error("custom provider '{provider}' could not be resolved: {reason}")]
    ProviderLookup { provider: String, reason: String },
    #[error("reading decision provider secrets failed: {0}")]
    Store(String),
    #[error("{provider} is not configured. Set {secret} in Settings > Secrets.")]
    MissingSecret {
        provider: String,
        secret: &'static str,
    },
    #[error("decision provider returned HTTP {status}: {body}")]
    Provider { status: u16, body: String },
    #[error("decision provider unreachable: {0}")]
    Transport(String),
    #[error("decision provider returned an unreadable response: {0}")]
    Decode(String),
}

/// Route boundary mapping. Caller mistakes are 400; a provider rejecting the
/// request itself (too many options for that model, bad schema) is 422; a
/// provider rate limit is 503; any other upstream failure is 502 with the
/// provider's message; a secret-store failure is an internal error.
impl From<DecisionError> for crate::ApiError {
    fn from(err: DecisionError) -> Self {
        use crate::ApiError;
        match err {
            DecisionError::InvalidModel(_)
            | DecisionError::InvalidRequest(_)
            | DecisionError::UnknownProvider(_)
            | DecisionError::MissingSecret { .. }
            | DecisionError::ProviderLookup { .. } => ApiError::BadRequest(err.to_string()),
            DecisionError::Provider {
                status: 400 | 422, ..
            } => ApiError::Unprocessable(err.to_string()),
            DecisionError::Provider { status: 429, .. } => {
                ApiError::ServiceUnavailable(err.to_string())
            }
            DecisionError::Provider { .. }
            | DecisionError::Transport(_)
            | DecisionError::Decode(_) => ApiError::BadGateway(err.to_string()),
            DecisionError::Store(_) => ApiError::Internal(anyhow::anyhow!(err.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TypeSafe API reference request example, plus a choice and a score.
    const DOCS_REQUEST: &str = r#"{
        "model": "typesafe/jev-latest",
        "state": "Help! My payouts have been failing for 3 days.",
        "questions": {
            "is_urgent": {"type": "noul", "instructions": "Does this convey urgency?",
                          "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}},
            "route": {"type": "choice", "instructions": "Which team?",
                      "criteria": {"technical": null, "billing": "Payouts, invoices", "sales": null}},
            "anger": {"type": "score", "instructions": "How angry?",
                      "criteria": ["Calm", "Frustrated", "Very angry"]}
        }
    }"#;

    /// TypeSafe API reference answer examples.
    const DOCS_RESPONSE: &str = r#"{
        "model": "jev-1.13.0",
        "answers": {
            "is_urgent": {"type": "noul", "noul": 0.95},
            "route": {"type": "choice", "choice": "billing",
                      "probabilities": {"billing": 0.88, "technical": 0.12, "sales": 0.0}, "confidence": 0.81},
            "anger": {"type": "score", "score": 1.05,
                      "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
                      "probabilities": {"0": 0.0, "1": 0.95, "2": 0.05}, "confidence": 0.92}
        },
        "usage": {"input_tokens": 41, "output_tokens": 0}
    }"#;

    #[test]
    fn docs_request_round_trips_preserving_order() {
        let req: DecisionRequest = serde_json::from_str(DOCS_REQUEST).unwrap();
        assert_eq!(
            req.questions.keys().collect::<Vec<_>>(),
            ["is_urgent", "route", "anger"]
        );
        let DecisionQuestion::Choice { criteria, .. } = &req.questions["route"] else {
            panic!("route should be a choice");
        };
        assert_eq!(
            criteria.keys().collect::<Vec<_>>(),
            ["technical", "billing", "sales"]
        );
        assert_eq!(criteria["technical"], None);
        let DecisionQuestion::Noul { criteria, .. } = &req.questions["is_urgent"] else {
            panic!("is_urgent should be a noul");
        };
        assert_eq!(
            criteria.as_ref().unwrap().yes.as_deref(),
            Some("Explicitly time-sensitive")
        );

        let back: DecisionRequest =
            serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
        assert_eq!(back, req);
        req.validate().unwrap();
    }

    #[test]
    fn docs_response_deserializes_every_answer_type() {
        let res: DecisionResponse = serde_json::from_str(DOCS_RESPONSE).unwrap();
        assert_eq!(res.usage.input_tokens, 41);
        assert!(matches!(res.answers["is_urgent"], DecisionAnswer::Noul { noul } if noul == 0.95));
        let DecisionAnswer::Choice {
            choice,
            probabilities,
            ..
        } = &res.answers["route"]
        else {
            panic!("route should be a choice answer");
        };
        assert_eq!(choice, "billing");
        assert_eq!(
            probabilities.keys().next().map(String::as_str),
            Some("billing")
        );
        let DecisionAnswer::Score { legend, .. } = &res.answers["anger"] else {
            panic!("anger should be a score answer");
        };
        assert_eq!(legend["1"], "Frustrated");
    }

    #[test]
    fn structured_state_and_missing_usage_are_accepted() {
        let req: DecisionRequest = serde_json::from_str(
            r#"{"model": "local/kev", "state": {"channel": "telegram", "text": "hi"},
                "questions": {"q": {"type": "noul", "instructions": "Greeting?"}}}"#,
        )
        .unwrap();
        assert!(matches!(req.state, DecisionContent::Structured(_)));
        req.validate().unwrap();

        let res: DecisionResponse =
            serde_json::from_str(r#"{"model": "m", "answers": {}}"#).unwrap();
        assert_eq!(res.usage, DecisionUsage::default());
    }

    fn request(questions: serde_json::Value) -> DecisionRequest {
        serde_json::from_value(serde_json::json!({
            "model": "typesafe/jev-latest", "state": "s", "questions": questions
        }))
        .unwrap()
    }

    #[test]
    fn validate_enforces_typesafe_limits() {
        assert!(request(serde_json::json!({})).validate().is_err());
        assert!(
            request(
                serde_json::json!({"q": {"type": "choice", "instructions": "i", "criteria": {}}})
            )
            .validate()
            .is_err()
        );
        let too_many: serde_json::Map<String, serde_json::Value> = (0..=MAX_CHOICE_OPTIONS)
            .map(|i| (format!("o{i}"), serde_json::Value::Null))
            .collect();
        assert!(request(serde_json::json!({"q": {"type": "choice", "instructions": "i", "criteria": too_many}}))
            .validate()
            .is_err());
        assert!(request(serde_json::json!({"q": {"type": "score", "instructions": "i", "criteria": ["only"]}}))
            .validate()
            .is_err());
        let eleven: Vec<String> = (0..11).map(|i| i.to_string()).collect();
        assert!(
            request(
                serde_json::json!({"q": {"type": "score", "instructions": "i", "criteria": eleven}})
            )
            .validate()
            .is_err()
        );
        request(serde_json::json!({"q": {"type": "score", "instructions": "i", "criteria": ["lo", "hi"]}}))
            .validate()
            .unwrap();
    }

    #[test]
    fn model_string_splits_on_first_slash_only() {
        let mut req = request(serde_json::json!({"q": {"type": "noul", "instructions": "i"}}));
        req.model = "aimlapi/typesafe/jev".into();
        assert_eq!(
            req.provider_and_model().unwrap(),
            ("aimlapi", "typesafe/jev")
        );
        for bad in ["jev-latest", "/jev", "typesafe/"] {
            req.model = bad.into();
            assert!(
                matches!(req.validate(), Err(DecisionError::InvalidModel(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn provider_endpoints() {
        assert_eq!(
            DecisionProviderKind::from_id("typesafe"),
            Some(DecisionProviderKind::Typesafe)
        );
        assert_eq!(DecisionProviderKind::from_id("my_llama"), None);
        let workers = DecisionProviderKind::WorkersAi;
        assert_eq!(workers.base_url(None), None);
        assert_eq!(
            workers.endpoint_url(&workers.base_url(Some("acc123")).unwrap(), "clef-flash"),
            "https://api.cloudflare.com/client/v4/accounts/acc123/ai/run/@cf/cloudflare/clef-flash"
        );
        let generic = DecisionProviderKind::SystemOne;
        assert_eq!(DecisionProviderKind::from_id("decisions"), Some(generic));
        assert!(!generic.api_key_required());
        assert_eq!(generic.base_url(Some("  ")), None);
        assert_eq!(
            generic.endpoint_url(
                &generic
                    .base_url(Some("https://decisions-dev.distri.dev/v1/"))
                    .unwrap(),
                "kev"
            ),
            "https://decisions-dev.distri.dev/v1/systemone"
        );
        assert_eq!(
            DecisionProviderKind::Aimlapi
                .endpoint_url("https://api.aimlapi.com/v1", "typesafe/jev"),
            "https://api.aimlapi.com/v1/decisions"
        );
        assert_eq!(
            DecisionProviderKind::WorkersAi.wire(),
            DecisionWire::CloudflareEnvelope
        );
        assert_eq!(
            systemone_url("http://127.0.0.1:8080/v1/"),
            "http://127.0.0.1:8080/v1/systemone"
        );
    }

    #[test]
    fn errors_map_to_api_statuses() {
        let status = |e: DecisionError| crate::ApiError::from(e).status();
        assert_eq!(status(DecisionError::InvalidModel("x".into())), 400);
        assert_eq!(
            status(DecisionError::MissingSecret {
                provider: "TypeSafe".into(),
                secret: "TYPESAFE_API_KEY"
            }),
            400
        );
        assert_eq!(
            status(DecisionError::Provider {
                status: 422,
                body: String::new()
            }),
            422
        );
        assert_eq!(
            status(DecisionError::Provider {
                status: 429,
                body: String::new()
            }),
            503
        );
        assert_eq!(
            status(DecisionError::Provider {
                status: 401,
                body: String::new()
            }),
            502
        );
        assert_eq!(status(DecisionError::Transport("down".into())), 502);
        assert_eq!(status(DecisionError::Store("db".into())), 500);
    }
}
