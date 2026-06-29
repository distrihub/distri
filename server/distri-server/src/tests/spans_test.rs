//! Integration tests for the spans/traces route handlers.
//!
//! These tests wire an `InMemorySpanStore` into an `AgentOrchestrator` and
//! exercise the `GET /spans` and `GET /traces` handlers via actix-web.

#[cfg(test)]
mod tests {
    use crate::stores::InMemorySpanStore;
    use actix_web::{test, web, App};
    use distri_core::initialize_stores;
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::api::spans::SpanRecord;
    use distri_types::configuration::{
        DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
    use distri_types::stores::SpanStore;
    use serde_json::{json, Value};
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

    async fn make_orchestrator_with_span_store(
        span_store: Arc<InMemorySpanStore>,
    ) -> Arc<distri_core::agent::AgentOrchestrator> {
        let mut stores = initialize_stores(&test_store_config())
            .await
            .expect("stores");
        stores.span_store = Some(span_store as Arc<dyn SpanStore>);

        Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .with_stores(stores)
                .build()
                .await
                .expect("orchestrator"),
        )
    }

    fn make_span(trace_id: &str, span_id: &str, parent_span_id: Option<&str>) -> SpanRecord {
        SpanRecord {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: parent_span_id.map(|s| s.to_string()),
            name: "test-span".to_string(),
            kind: 1,
            start_time_ns: 1_000_000_000,
            end_time_ns: 2_000_000_000,
            attributes: json!({}),
            events: json!([]),
            status_code: 0,
            status_message: None,
            resource: json!({}),
            scope_name: None,
        }
    }

    // ── GET /spans?trace_id=X ─────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_spans_by_trace_id_returns_inserted_span() {
        let span_store = InMemorySpanStore::new();
        let trace_id = uuid::Uuid::new_v4().to_string();
        let span = make_span(&trace_id, "span-1", None);
        span_store.bulk_insert(vec![span]).await.expect("insert");

        let orchestrator = make_orchestrator_with_span_store(span_store).await;
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
            .uri(&format!("/v1/spans?trace_id={}", trace_id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200, "expected 200 got {}", resp.status());

        let body: Value = test::read_body_json(resp).await;
        let spans = body["spans"].as_array().expect("spans array");
        assert_eq!(spans.len(), 1, "should have 1 span");
        assert_eq!(spans[0]["traceId"], trace_id);
        assert_eq!(spans[0]["spanId"], "span-1");
    }

    // ── GET /spans — missing params ────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_spans_missing_query_param_returns_400() {
        let span_store = InMemorySpanStore::new();
        let orchestrator = make_orchestrator_with_span_store(span_store).await;
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

        let req = test::TestRequest::get().uri("/v1/spans").to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 400);
    }

    // ── GET /traces ────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_traces_returns_inserted_trace() {
        let span_store = InMemorySpanStore::new();
        let trace_id = uuid::Uuid::new_v4().to_string();
        // Root span (no parent)
        let root_span = make_span(&trace_id, "root-span", None);
        // Child span
        let child_span = make_span(&trace_id, "child-span", Some("root-span"));
        span_store
            .bulk_insert(vec![root_span, child_span])
            .await
            .expect("insert");

        let orchestrator = make_orchestrator_with_span_store(span_store).await;
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

        let req = test::TestRequest::get().uri("/v1/traces").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200, "expected 200 got {}", resp.status());

        let body: Value = test::read_body_json(resp).await;
        let traces = body["traces"].as_array().expect("traces array");
        assert!(!traces.is_empty(), "should have at least one trace");
        let trace = traces.iter().find(|t| t["traceId"] == trace_id);
        assert!(trace.is_some(), "should find our trace_id in results");
        assert_eq!(
            trace.unwrap()["spanCount"],
            2,
            "should aggregate both spans into one trace"
        );
    }

    // ── GET /traces?limit=1 ────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_traces_respects_limit() {
        let span_store = InMemorySpanStore::new();
        // Insert root spans for 3 different traces
        for i in 0i64..3 {
            let trace_id = format!("trace-limit-test-{}", i);
            let span = SpanRecord {
                trace_id,
                span_id: format!("span-{}", i),
                parent_span_id: None,
                name: format!("trace-{}", i),
                kind: 1,
                start_time_ns: i * 1_000_000_000,
                end_time_ns: i * 1_000_000_000 + 500_000_000,
                attributes: json!({}),
                events: json!([]),
                status_code: 0,
                status_message: None,
                resource: json!({}),
                scope_name: None,
            };
            span_store.bulk_insert(vec![span]).await.expect("insert");
        }

        let orchestrator = make_orchestrator_with_span_store(span_store).await;
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
            .uri("/v1/traces?limit=1")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = test::read_body_json(resp).await;
        let traces = body["traces"].as_array().expect("traces array");
        assert_eq!(traces.len(), 1, "limit=1 should return exactly 1 trace");
    }
}
