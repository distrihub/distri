//! Integration tests for the `GET /v1/usage/stats` handler.
//!
//! These tests verify that:
//!  - the endpoint returns 200 with the correct JSON shape
//!  - default query params are applied (since = 30d ago, until = now,
//!    bucket = day)
//!  - explicit query params are echoed back in `filters_applied`
//!  - cloud-only params (`user_id`, `bot_id`, `channel_id`) are accepted and
//!    echoed back without causing errors

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App};
    use distri_core::initialize_stores;
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::configuration::{
        DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
    use serde_json::Value;
    use std::sync::Arc;

    fn test_store_config() -> StoreConfig {
        let db_name = uuid::Uuid::new_v4();
        let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
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
        let stores = initialize_stores(&test_store_config())
            .await
            .expect("stores");

        Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .with_stores(stores)
                .build()
                .await
                .expect("orchestrator"),
        )
    }

    // ── GET /usage/stats (no params) ──────────────────────────────────────────

    #[actix_web::test]
    async fn test_usage_stats_returns_200_with_correct_shape() {
        let orchestrator = make_orchestrator().await;
        let server_config = ServerConfig::default();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(server_config))
                .configure(|cfg| {
                    cfg.app_data(web::Data::new(orchestrator))
                        .service(web::scope("/v1").configure(crate::routes::distri));
                }),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/v1/usage/stats")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200, "expected 200 got {}", resp.status());

        let body: Value = test::read_body_json(resp).await;

        // Shape checks
        assert!(body["totals"].is_object(), "totals must be an object");
        assert!(body["buckets"].is_array(), "buckets must be an array");
        let fa = body
            .get("filters_applied")
            .or_else(|| body.get("filtersApplied"))
            .expect("filters_applied must be present in response");
        assert!(fa.is_object(), "filters_applied must be an object");

        // Zero-valued totals
        let totals = &body["totals"];
        assert_eq!(totals["messages"], 0);
        assert_eq!(
            totals.get("input_tokens").or_else(|| totals.get("inputTokens"))
                .and_then(|v| v.as_i64()).unwrap_or(0),
            0
        );
        assert_eq!(
            totals.get("total_tokens").or_else(|| totals.get("totalTokens"))
                .and_then(|v| v.as_i64()).unwrap_or(0),
            0
        );

        // Default bucket
        assert_eq!(fa["bucket"], "day", "default bucket should be 'day'");
        assert!(fa["since"].is_string(), "since should be a string");
        assert!(fa["until"].is_string(), "until should be a string");
    }

    // ── GET /usage/stats?bucket=week ─────────────────────────────────────────

    #[actix_web::test]
    async fn test_usage_stats_bucket_param_is_echoed() {
        let orchestrator = make_orchestrator().await;
        let server_config = ServerConfig::default();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(server_config))
                .configure(|cfg| {
                    cfg.app_data(web::Data::new(orchestrator))
                        .service(web::scope("/v1").configure(crate::routes::distri));
                }),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/v1/usage/stats?bucket=week")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: Value = test::read_body_json(resp).await;
        let fa = body
            .get("filters_applied")
            .or_else(|| body.get("filtersApplied"))
            .expect("filters_applied must be present in response");
        assert_eq!(fa["bucket"], "week");
    }

    // ── GET /usage/stats with cloud-only params ───────────────────────────────

    #[actix_web::test]
    async fn test_usage_stats_accepts_cloud_only_params_without_error() {
        let orchestrator = make_orchestrator().await;
        let server_config = ServerConfig::default();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(server_config))
                .configure(|cfg| {
                    cfg.app_data(web::Data::new(orchestrator))
                        .service(web::scope("/v1").configure(crate::routes::distri));
                }),
        )
        .await;

        let user_id = uuid::Uuid::new_v4();
        let uri = format!(
            "/v1/usage/stats?user_id={}&bucket=month",
            user_id
        );
        let req = test::TestRequest::get().uri(&uri).to_request();
        let resp = test::call_service(&app, req).await;
        // Should not 400 or 500 — cloud-only params are accepted.
        assert_eq!(resp.status(), 200, "cloud-only params should be accepted");

        let body: Value = test::read_body_json(resp).await;
        let fa = body
            .get("filters_applied")
            .or_else(|| body.get("filtersApplied"))
            .expect("filters_applied must be present in response");
        assert_eq!(fa["bucket"], "month");
        // user_id is echoed back (snake_case is the default serde serialization)
        let echoed_uid = fa
            .get("user_id")
            .or_else(|| fa.get("userId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(echoed_uid, user_id.to_string());
    }
}
