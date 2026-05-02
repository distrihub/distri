//! Integration tests for the connections route module.
//!
//! These tests wire up an in-memory `ConnectionStore` and `ConnectionTokenStore`
//! into an `AgentOrchestrator`, then exercise the CRUD handlers via actix-web test app.

#[cfg(test)]
mod tests {
    use crate::stores::{InMemoryConnectionStore, InMemoryConnectionTokenStore};
    use actix_web::{test, web, App};
    use distri_core::initialize_stores;
    use distri_core::AgentOrchestratorBuilder;
    use distri_types::configuration::{
        DbConnectionConfig, MetadataStoreConfig, ServerConfig, StoreConfig,
    };
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

    async fn make_orchestrator_with_conn_stores() -> Arc<distri_core::agent::AgentOrchestrator> {
        let conn_store = InMemoryConnectionStore::new();
        let token_store = InMemoryConnectionTokenStore::new();

        let mut stores = initialize_stores(&test_store_config())
            .await
            .expect("stores");
        stores.connection_store =
            Some(conn_store as Arc<dyn distri_types::stores::ConnectionStore>);
        stores.connection_token_store =
            Some(token_store as Arc<dyn distri_types::stores::ConnectionTokenStore>);

        Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .with_stores(stores)
                .build()
                .await
                .expect("orchestrator"),
        )
    }

    // ── POST /connections ─────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_create_custom_connection_returns_connected() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        let req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "my-api",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [
                        { "key": "TOKEN", "is_secret": true, "required": true }
                    ]
                },
                "skill_content": "# my-api\nDoes things.",
                "secrets": { "TOKEN": "sk-test-123" }
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200, "expected 200 got {}", resp.status());

        let body: Value = test::read_body_json(resp).await;
        assert_eq!(
            body["type"], "connected",
            "response type should be 'connected'"
        );
        assert!(
            body["connection"]["id"].is_string(),
            "should have connection id"
        );
        assert_eq!(body["connection"]["name"], "my-api");
    }

    // ── GET /connections ──────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_connections_returns_created_entry() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        // Create a connection first
        let create_req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "list-test",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [{"key": "KEY", "is_secret": true, "required": true}]
                },
                "skill_content": "# list-test\n",
                "secrets": { "KEY": "val" }
            }))
            .to_request();
        assert_eq!(test::call_service(&app, create_req).await.status(), 200);

        // Now list
        let list_req = test::TestRequest::get().uri("/v1/connections").to_request();
        let list_resp = test::call_service(&app, list_req).await;
        assert_eq!(list_resp.status(), 200);

        let body: Value = test::read_body_json(list_resp).await;
        let arr = body.as_array().expect("list should return an array");
        assert!(
            arr.iter().any(|c| c["name"] == "list-test"),
            "should find created connection"
        );
    }

    // ── GET /connections/{id} ─────────────────────────────────────────────

    #[actix_web::test]
    async fn test_get_connection_by_id() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        // Create
        let create_req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "get-test",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [{"key": "KEY", "is_secret": true, "required": true}]
                },
                "skill_content": "# get-test\n",
                "secrets": {}
            }))
            .to_request();
        let created: Value = test::read_body_json(test::call_service(&app, create_req).await).await;
        let conn_id = created["connection"]["id"]
            .as_str()
            .expect("id")
            .to_string();

        // Get by id
        let get_req = test::TestRequest::get()
            .uri(&format!("/v1/connections/{}", conn_id))
            .to_request();
        let get_resp = test::call_service(&app, get_req).await;
        assert_eq!(get_resp.status(), 200);

        let body: Value = test::read_body_json(get_resp).await;
        assert_eq!(body["id"], conn_id);
        assert_eq!(body["name"], "get-test");
    }

    // ── GET /connections/{id} - 404 ───────────────────────────────────────

    #[actix_web::test]
    async fn test_get_nonexistent_connection_returns_404() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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
            .uri("/v1/connections/00000000-0000-0000-0000-000000000001")
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 404);
    }

    // ── PATCH /connections/{id} ───────────────────────────────────────────

    #[actix_web::test]
    async fn test_update_connection_name() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        // Create
        let create_req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "original-name",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [{"key": "KEY", "is_secret": true, "required": true}]
                },
                "skill_content": "# original\n",
                "secrets": {}
            }))
            .to_request();
        let created: Value = test::read_body_json(test::call_service(&app, create_req).await).await;
        let conn_id = created["connection"]["id"]
            .as_str()
            .expect("id")
            .to_string();

        // Patch name
        let patch_req = test::TestRequest::patch()
            .uri(&format!("/v1/connections/{}", conn_id))
            .set_json(json!({"name": "new-name"}))
            .to_request();
        let patch_resp = test::call_service(&app, patch_req).await;
        assert_eq!(patch_resp.status(), 200);

        let body: Value = test::read_body_json(patch_resp).await;
        assert_eq!(body["name"], "new-name");
    }

    // ── DELETE /connections/{id} ──────────────────────────────────────────

    #[actix_web::test]
    async fn test_delete_connection() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        // Create
        let create_req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "del-test",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [{"key": "KEY", "is_secret": true, "required": true}]
                },
                "skill_content": "# del-test\n",
                "secrets": {}
            }))
            .to_request();
        let created: Value = test::read_body_json(test::call_service(&app, create_req).await).await;
        let conn_id = created["connection"]["id"]
            .as_str()
            .expect("id")
            .to_string();

        // Delete
        let del_req = test::TestRequest::delete()
            .uri(&format!("/v1/connections/{}", conn_id))
            .to_request();
        let del_resp = test::call_service(&app, del_req).await;
        assert_eq!(del_resp.status(), 200);

        let body: Value = test::read_body_json(del_resp).await;
        assert_eq!(body["deleted"], true);
        assert_eq!(body["connection_id"], conn_id);

        // Verify it's gone
        let get_req = test::TestRequest::get()
            .uri(&format!("/v1/connections/{}", conn_id))
            .to_request();
        assert_eq!(test::call_service(&app, get_req).await.status(), 404);
    }

    // ── Validation ────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_create_connection_name_too_long_returns_400() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        let long_name: String = "a".repeat(65);
        let req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": long_name,
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": [{"key": "KEY", "is_secret": true, "required": true}]
                },
                "skill_content": "# long\n",
                "secrets": {}
            }))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 400);
    }

    #[actix_web::test]
    async fn test_create_connection_empty_custom_fields_returns_400() {
        let orchestrator = make_orchestrator_with_conn_stores().await;
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

        let req = test::TestRequest::post()
            .uri("/v1/connections")
            .set_json(json!({
                "name": "empty-fields",
                "auth_scope": "workspace",
                "auth_type": {
                    "type": "custom",
                    "fields": []
                },
                "skill_content": "# empty\n",
                "secrets": {}
            }))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 400);
    }

    // ── Store-not-configured path ─────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_connections_without_store_returns_503() {
        // Build orchestrator WITHOUT a connection store (the default)
        let orchestrator = Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .build()
                .await
                .expect("orchestrator"),
        );
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

        let req = test::TestRequest::get().uri("/v1/connections").to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 503);
    }
}
