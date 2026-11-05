//! `POST /v1/llm/execute` when the caller names no model, through the real
//! route table and real (SQLite) stores, with a fake upstream behind a
//! `custom_*` provider saved as the default model.

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use distri_core::initialize_stores;
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::configuration::{
        DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
    use distri_types::stores::{CustomProviderConfig, UpsertProviderRequest};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const NO_DEFAULT: &str = "no default model is set for the workspace";

    async fn make_orchestrator() -> Arc<distri_core::agent::AgentOrchestrator> {
        let db_url = format!("file:{}?mode=memory&cache=shared", uuid::Uuid::new_v4());
        let config = StoreConfig {
            metadata: MetadataStoreConfig {
                db_config: Some(DbConnectionConfig {
                    database_url: db_url,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
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

    /// Saves a custom provider and `custom_llama/default-model` as the default.
    async fn set_default_model(
        orchestrator: &distri_core::agent::AgentOrchestrator,
        base_url: &str,
    ) {
        orchestrator
            .stores
            .provider_store
            .as_ref()
            .expect("provider store")
            .upsert_provider(UpsertProviderRequest {
                provider_id: "custom_llama".into(),
                secrets: HashMap::from([("CUSTOM_LLAMA_API_KEY".to_string(), "k".to_string())]),
                config: Some(CustomProviderConfig {
                    id: "custom_llama".into(),
                    name: "Fake upstream".into(),
                    base_url: base_url.into(),
                    project_id: None,
                }),
                custom_models: None,
                default_model: Some("custom_llama/default-model".into()),
                connection_provider: None,
            })
            .await
            .expect("upsert provider");
    }

    /// An upstream that only answers for the default model and, when asked
    /// to, only with the request's temperature.
    async fn upstream_expecting(body: Value) -> MockServer {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-1",
                "object": "chat.completion",
                "created": 1,
                "model": "default-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "ran on the default"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })))
            .expect(1)
            .mount(&upstream)
            .await;
        upstream
    }

    fn request(model_settings: Option<Value>) -> Value {
        let mut body = json!({
            "messages": [{"role": "user", "name": null, "parts": [{"part_type": "text", "data": "hi"}]}]
        });
        if let Some(ms) = model_settings {
            body["model_settings"] = ms;
        }
        body
    }

    async fn post(
        orchestrator: Arc<distri_core::agent::AgentOrchestrator>,
        body: Value,
    ) -> (u16, Value) {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ServerConfig::default()))
                .app_data(web::Data::new(orchestrator.clone()))
                .service(web::scope("/v1").configure(crate::routes::distri)),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/v1/llm/execute")
            .set_json(body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16();
        let bytes = test::read_body(resp).await;
        let parsed = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, parsed)
    }

    #[actix_web::test]
    async fn partial_model_settings_run_on_the_default_model() {
        let upstream =
            upstream_expecting(json!({"model": "default-model", "temperature": 0.25})).await;
        let orchestrator = make_orchestrator().await;
        set_default_model(&orchestrator, &format!("{}/v1", upstream.uri())).await;

        let (status, body) = post(orchestrator, request(Some(json!({"temperature": 0.25})))).await;

        assert_eq!(status, 200, "{body}");
        assert_eq!(body["content"], "ran on the default");
    }

    #[actix_web::test]
    async fn empty_model_settings_run_on_the_default_model() {
        let upstream = upstream_expecting(json!({"model": "default-model"})).await;
        let orchestrator = make_orchestrator().await;
        set_default_model(&orchestrator, &format!("{}/v1", upstream.uri())).await;

        let (status, body) = post(orchestrator, request(Some(json!({})))).await;

        assert_eq!(status, 200, "{body}");
    }

    #[actix_web::test]
    async fn no_model_settings_still_run_on_the_default_model() {
        let upstream = upstream_expecting(json!({"model": "default-model"})).await;
        let orchestrator = make_orchestrator().await;
        set_default_model(&orchestrator, &format!("{}/v1", upstream.uri())).await;

        let (status, body) = post(orchestrator, request(None)).await;

        assert_eq!(status, 200, "{body}");
    }

    #[actix_web::test]
    async fn no_model_anywhere_says_no_default_is_set() {
        for model_settings in [None, Some(json!({})), Some(json!({"temperature": 0.25}))] {
            let orchestrator = make_orchestrator().await;
            let (status, body) = post(orchestrator, request(model_settings.clone())).await;
            assert_eq!(status, 400, "{model_settings:?}: {body}");
            assert!(
                body["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(NO_DEFAULT),
                "{model_settings:?}: {body}"
            );
        }
    }

    /// With no default to merge onto, a request that names its model and
    /// provider runs as it always did.
    #[actix_web::test]
    async fn a_named_model_needs_no_default() {
        let upstream = upstream_expecting(json!({"model": "named-model"})).await;
        let orchestrator = make_orchestrator().await;

        let (status, body) = post(
            orchestrator,
            request(Some(json!({
                "model": "named-model",
                "provider": {
                    "name": "openai_compat",
                    "base_url": format!("{}/v1", upstream.uri()),
                    "api_key": "k"
                }
            }))),
        )
        .await;

        assert_eq!(status, 200, "{body}");
    }
}
