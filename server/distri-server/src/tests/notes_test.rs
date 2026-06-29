//! Integration tests for the notes route module.
//!
//! Uses `InMemoryNoteStore` wired into an `AgentOrchestrator` to exercise all
//! CRUD handlers via the actix-web test harness.

#[cfg(test)]
mod tests {
    use crate::stores::InMemoryNoteStore;
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

    async fn make_orchestrator() -> Arc<distri_core::agent::AgentOrchestrator> {
        let note_store = InMemoryNoteStore::new();
        let mut stores = initialize_stores(&test_store_config())
            .await
            .expect("stores");
        stores.note_store = Some(note_store as Arc<dyn distri_types::stores::NoteStore>);

        Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .with_stores(stores)
                .build()
                .await
                .expect("orchestrator"),
        )
    }

    // ── POST /notes ───────────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_create_note_returns_201() {
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

        let req = test::TestRequest::post()
            .uri("/v1/notes")
            .set_json(json!({
                "title": "Hello",
                "content": "World",
                "tags": ["test"]
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 201, "expected 201 got {}", resp.status());

        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["title"], "Hello");
        assert_eq!(body["content"], "World");
        assert!(body["id"].is_string());
        assert_eq!(body["tags"][0], "test");
    }

    // ── GET /notes ────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_notes_returns_created_entry() {
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

        // Create first
        let create_req = test::TestRequest::post()
            .uri("/v1/notes")
            .set_json(json!({"title": "List Test", "content": "body", "tags": []}))
            .to_request();
        assert_eq!(test::call_service(&app, create_req).await.status(), 201);

        // List
        let list_req = test::TestRequest::get().uri("/v1/notes").to_request();
        let list_resp = test::call_service(&app, list_req).await;
        assert_eq!(list_resp.status(), 200);

        let body: Value = test::read_body_json(list_resp).await;
        let notes = body["notes"].as_array().expect("notes array");
        assert!(
            notes.iter().any(|n| n["title"] == "List Test"),
            "should find created note"
        );
    }

    // ── GET /notes/{id} ───────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_get_note_by_id() {
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

        // Create
        let created: Value = test::read_body_json(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/v1/notes")
                    .set_json(json!({"title": "Get Test", "content": "body", "tags": []}))
                    .to_request(),
            )
            .await,
        )
        .await;
        let id = created["id"].as_str().expect("id").to_string();

        // Get by id
        let get_resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/v1/notes/{}", id))
                .to_request(),
        )
        .await;
        assert_eq!(get_resp.status(), 200);

        let body: Value = test::read_body_json(get_resp).await;
        assert_eq!(body["id"], id);
        assert_eq!(body["title"], "Get Test");
    }

    // ── PUT /notes/{id} ───────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_update_note_title() {
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

        // Create
        let created: Value = test::read_body_json(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/v1/notes")
                    .set_json(json!({"title": "Original", "content": "body", "tags": []}))
                    .to_request(),
            )
            .await,
        )
        .await;
        let id = created["id"].as_str().expect("id").to_string();

        // Update
        let update_resp = test::call_service(
            &app,
            test::TestRequest::put()
                .uri(&format!("/v1/notes/{}", id))
                .set_json(json!({"title": "Updated"}))
                .to_request(),
        )
        .await;
        assert_eq!(update_resp.status(), 200);

        let body: Value = test::read_body_json(update_resp).await;
        assert_eq!(body["title"], "Updated");
        assert_eq!(body["content"], "body", "content unchanged");
    }

    // ── DELETE /notes/{id} → 204 ──────────────────────────────────────────

    #[actix_web::test]
    async fn test_delete_note_returns_204() {
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

        // Create
        let created: Value = test::read_body_json(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/v1/notes")
                    .set_json(json!({"title": "Del", "content": "body", "tags": []}))
                    .to_request(),
            )
            .await,
        )
        .await;
        let id = created["id"].as_str().expect("id").to_string();

        let del_resp = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri(&format!("/v1/notes/{}", id))
                .to_request(),
        )
        .await;
        assert_eq!(del_resp.status(), 204);
    }

    // ── GET after DELETE → 404 ────────────────────────────────────────────

    #[actix_web::test]
    async fn test_get_deleted_note_returns_404() {
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

        // Create then delete
        let created: Value = test::read_body_json(
            test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/v1/notes")
                    .set_json(json!({"title": "Gone", "content": "body", "tags": []}))
                    .to_request(),
            )
            .await,
        )
        .await;
        let id = created["id"].as_str().expect("id").to_string();

        test::call_service(
            &app,
            test::TestRequest::delete()
                .uri(&format!("/v1/notes/{}", id))
                .to_request(),
        )
        .await;

        // Get again → 404
        let get_resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/v1/notes/{}", id))
                .to_request(),
        )
        .await;
        assert_eq!(get_resp.status(), 404);
    }

    // ── Tag filter ────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn test_list_notes_tag_filter() {
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

        // Two notes with different tags
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/notes")
                .set_json(json!({"title": "Alpha", "content": "a", "tags": ["alpha"]}))
                .to_request(),
        )
        .await;
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/v1/notes")
                .set_json(json!({"title": "Beta", "content": "b", "tags": ["beta"]}))
                .to_request(),
        )
        .await;

        // Filter by tag=alpha
        let list_resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/notes?tag=alpha")
                .to_request(),
        )
        .await;
        assert_eq!(list_resp.status(), 200);

        let body: Value = test::read_body_json(list_resp).await;
        let notes = body["notes"].as_array().expect("notes array");
        assert_eq!(notes.len(), 1, "should only return notes with tag alpha");
        assert_eq!(notes[0]["title"], "Alpha");
    }

    // ── GET nonexistent → 404 ─────────────────────────────────────────────

    #[actix_web::test]
    async fn test_get_nonexistent_note_returns_404() {
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

        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/v1/notes/00000000-0000-0000-0000-000000000001")
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 404);
    }
}
