//! `POST /v1/decisions` through the real route table and real (SQLite)
//! stores. A self-hosted decision model is a `custom_*` provider whose
//! `base_url` serves `/systemone`, registered exactly as the UI/CLI would.
//!
//! `decisions_against_live_llama_server` runs only when
//! `DISTRI_DECISION_LIVE_URL` points at a llama-server ≥ b11361 serving a
//! decision model (e.g. `https://decisions-dev.distri.dev/v1`), with its key
//! in `DISTRI_DECISION_LIVE_KEY` when the server requires one.

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use distri_core::initialize_stores;
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::configuration::{
        DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
    use distri_types::decision::{DecisionAnswer, DecisionResponse};
    use distri_types::stores::{CustomProviderConfig, UpsertProviderRequest};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use wiremock::matchers::{bearer_token, body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_store_config() -> StoreConfig {
        let db_url = format!("file:{}?mode=memory&cache=shared", uuid::Uuid::new_v4());
        StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn make_orchestrator() -> Arc<distri_core::agent::AgentOrchestrator> {
        let config = test_store_config();
        let stores = initialize_stores(&config).await.expect("stores");
        Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(config)
                .with_stores(stores)
                .build()
                .await
                .expect("orchestrator"),
        )
    }

    async fn register_custom_provider(
        orchestrator: &distri_core::agent::AgentOrchestrator,
        base_url: &str,
        api_key: Option<&str>,
    ) {
        let secrets: HashMap<String, String> = api_key
            .map(|k| HashMap::from([("CUSTOM_LLAMA_API_KEY".to_string(), k.to_string())]))
            .unwrap_or_default();
        orchestrator
            .stores
            .provider_store
            .as_ref()
            .expect("provider store")
            .upsert_provider(UpsertProviderRequest {
                provider_id: "custom_llama".into(),
                secrets,
                config: Some(CustomProviderConfig {
                    id: "custom_llama".into(),
                    name: "Local llama-server".into(),
                    base_url: base_url.into(),
                    project_id: None,
                }),
                custom_models: None,
                default_model: None,
                connection_provider: None,
            })
            .await
            .expect("upsert custom provider");
    }

    fn body(model: &str) -> Value {
        json!({
            "model": model,
            "state": {"channel": "telegram",
                      "user_message": "can you pull last month invoices from stripe and email the totals to finance@acme.com"},
            "questions": {
                "route": {"type": "choice", "instructions": "Which agent should handle this message?",
                          "criteria": {"support": "Product questions, bugs, account help",
                                       "finance_ops": "Billing, invoices, payments, Stripe, accounting",
                                       "research": "Web research, summarising sources",
                                       "scheduler": "Calendar, meetings, reminders"}},
                "needs_approval": {"type": "noul",
                                   "instructions": "Does this request send data outside the workspace and need human approval?"}
            }
        })
    }

    macro_rules! app {
        ($orchestrator:expr) => {
            test::init_service(
                App::new()
                    .app_data(web::Data::new(ServerConfig::default()))
                    .configure(|cfg| {
                        // Registered exactly as `agent_server` does.
                        if let Some(store) = $orchestrator.stores.provider_store.clone() {
                            cfg.app_data(web::Data::new(store));
                        }
                        cfg.app_data(web::Data::new($orchestrator.clone()))
                            .service(web::scope("/v1").configure(crate::routes::distri));
                    }),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn custom_provider_round_trip() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(bearer_token("local-key"))
            .and(body_partial_json(json!({"model": "kev-0.8b"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "kev-0.8b",
                "answers": {
                    "route": {"type": "choice", "choice": "finance_ops",
                              "probabilities": {"support": 0.04, "finance_ops": 0.93, "research": 0.02, "scheduler": 0.01},
                              "confidence": 0.9},
                    "needs_approval": {"type": "noul", "noul": 0.46}
                },
                "usage": {"input_tokens": 120, "output_tokens": 0}
            })))
            .expect(1)
            .mount(&upstream)
            .await;

        let orchestrator = make_orchestrator().await;
        register_custom_provider(
            &orchestrator,
            &format!("{}/v1", upstream.uri()),
            Some("local-key"),
        )
        .await;
        let app = app!(orchestrator);

        let req = test::TestRequest::post()
            .uri("/v1/decisions")
            .set_json(body("custom_llama/kev-0.8b"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let res: DecisionResponse = test::read_body_json(resp).await;
        assert_eq!(res.usage.input_tokens, 120);
        assert!(
            matches!(&res.answers["route"], DecisionAnswer::Choice { choice, .. } if choice == "finance_ops")
        );
        assert!(matches!(
            res.answers["needs_approval"],
            DecisionAnswer::Noul { .. }
        ));
    }

    /// `llamacpp` is a built-in provider: its base URL and key are ordinary
    /// provider settings (what the Models UI saves), no custom provider needed.
    async fn configure_llamacpp(
        orchestrator: &distri_core::agent::AgentOrchestrator,
        base_url: &str,
        api_key: Option<&str>,
    ) {
        let mut secrets = HashMap::from([("LLAMACPP_BASE_URL".to_string(), base_url.to_string())]);
        if let Some(k) = api_key {
            secrets.insert("LLAMACPP_API_KEY".to_string(), k.to_string());
        }
        orchestrator
            .stores
            .provider_store
            .as_ref()
            .expect("provider store")
            .upsert_provider(UpsertProviderRequest {
                provider_id: "llamacpp".into(),
                secrets,
                config: None,
                custom_models: None,
                default_model: None,
                connection_provider: None,
            })
            .await
            .expect("configure llamacpp");
    }

    #[actix_web::test]
    async fn llamacpp_provider_is_configured_like_any_other() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(bearer_token("dev-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "kev",
                "answers": {
                    "route": {"type": "choice", "choice": "finance_ops",
                              "probabilities": {"support": 0.04, "finance_ops": 0.93, "research": 0.02, "scheduler": 0.01},
                              "confidence": 0.9},
                    "needs_approval": {"type": "noul", "noul": 0.46}
                },
                "usage": {"input_tokens": 120, "output_tokens": 0}
            })))
            // one decision + one provider test probe
            .expect(2)
            .mount(&upstream)
            .await;

        let orchestrator = make_orchestrator().await;
        configure_llamacpp(
            &orchestrator,
            &format!("{}/v1", upstream.uri()),
            Some("dev-key"),
        )
        .await;
        let app = app!(orchestrator);

        // Listed with its two settings, so the UI renders a form for it.
        let req = test::TestRequest::get().uri("/v1/providers").to_request();
        let providers: Value = test::read_body_json(test::call_service(&app, req).await).await;
        let llama = providers
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "llamacpp")
            .expect("llamacpp listed");
        let keys: Vec<&str> = llama["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["LLAMACPP_BASE_URL", "LLAMACPP_API_KEY"]);

        let req = test::TestRequest::post()
            .uri("/v1/decisions")
            .set_json(body("llamacpp/kev"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let res: DecisionResponse = test::read_body_json(resp).await;
        assert!(
            matches!(&res.answers["route"], DecisionAnswer::Choice { choice, .. } if choice == "finance_ops")
        );

        // The UI's "Test" button: POSTs a one-question decision to the server.
        let req = test::TestRequest::post()
            .uri("/v1/providers/test")
            .set_json(json!({"provider_id": "llamacpp"}))
            .to_request();
        let probe: Value = test::read_body_json(test::call_service(&app, req).await).await;
        assert_eq!(probe["ok"], true, "{probe}");
    }

    #[actix_web::test]
    async fn llamacpp_without_base_url_is_400_naming_the_setting() {
        let (status, error) = error_for("llamacpp/kev", None).await;
        assert_eq!(status, 400);
        assert!(error.contains("LLAMACPP_BASE_URL"), "{error}");
    }

    async fn error_for(model: &str, body_override: Option<Value>) -> (u16, String) {
        let orchestrator = make_orchestrator().await;
        let app = app!(orchestrator);
        let req = test::TestRequest::post()
            .uri("/v1/decisions")
            .set_json(body_override.unwrap_or_else(|| body(model)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16();
        let body: Value = test::read_body_json(resp).await;
        (
            status,
            body["error"].as_str().unwrap_or_default().to_string(),
        )
    }

    #[actix_web::test]
    async fn missing_hosted_key_is_400_naming_the_secret() {
        let (status, error) = error_for("typesafe/jev-latest", None).await;
        assert_eq!(status, 400);
        assert!(error.contains("TYPESAFE_API_KEY"), "{error}");
    }

    #[actix_web::test]
    async fn unknown_and_unconfigured_providers_are_400() {
        let (status, error) = error_for("openai/gpt-5", None).await;
        assert_eq!(status, 400);
        assert!(error.contains("unknown decision provider"), "{error}");

        let (status, error) = error_for("custom_nowhere/kev", None).await;
        assert_eq!(status, 400);
        assert!(error.contains("custom_nowhere"), "{error}");
    }

    #[actix_web::test]
    async fn out_of_range_score_is_400() {
        let bad = json!({"model": "typesafe/jev-latest", "state": "s",
                         "questions": {"q": {"type": "score", "instructions": "i", "criteria": ["one"]}}});
        let (status, error) = error_for("", Some(bad)).await;
        assert_eq!(status, 400);
        assert!(error.contains("2 to 10 levels"), "{error}");
    }

    #[actix_web::test]
    async fn upstream_failure_is_502() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("model crashed"))
            .mount(&upstream)
            .await;
        let orchestrator = make_orchestrator().await;
        register_custom_provider(&orchestrator, &format!("{}/v1", upstream.uri()), None).await;
        let app = app!(orchestrator);
        let req = test::TestRequest::post()
            .uri("/v1/decisions")
            .set_json(body("custom_llama/kev"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 502);
        let body: Value = test::read_body_json(resp).await;
        assert!(body["error"].as_str().unwrap().contains("model crashed"));
    }

    #[actix_web::test]
    async fn decisions_against_live_llama_server() {
        let Ok(base_url) = std::env::var("DISTRI_DECISION_LIVE_URL") else {
            eprintln!("DISTRI_DECISION_LIVE_URL not set; skipping live llama-server test");
            return;
        };
        let orchestrator = make_orchestrator().await;
        configure_llamacpp(
            &orchestrator,
            &base_url,
            std::env::var("DISTRI_DECISION_LIVE_KEY").ok().as_deref(),
        )
        .await;
        let app = app!(orchestrator);

        let req = test::TestRequest::post()
            .uri("/v1/decisions")
            .set_json(body("llamacpp/kev"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let res: DecisionResponse = test::read_body_json(resp).await;
        assert!(res.usage.input_tokens > 0);
        let DecisionAnswer::Choice {
            choice,
            probabilities,
            ..
        } = &res.answers["route"]
        else {
            panic!("route should be a choice");
        };
        assert_eq!(choice, "finance_ops");
        let sum: f64 = probabilities.values().sum();
        assert!((sum - 1.0).abs() < 1e-3);
    }
}
