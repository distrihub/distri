use actix_web::{test, web, App, HttpResponse};
use distri::types::ServerConfig;
use distri_a2a::{APIKeySecurityScheme, AgentProvider, SecurityScheme};
use std::collections::HashMap;

use super::{create_security_middleware, SecurityContext};

#[actix_web::test]
async fn test_api_key_authentication() {
    // Create a server config with API key security
    let mut security_schemes = HashMap::new();
    security_schemes.insert(
        "apiKey".to_string(),
        SecurityScheme::ApiKey(APIKeySecurityScheme {
            name: "X-API-Key".to_string(),
            location: "header".to_string(),
            description: Some("API key for authentication".to_string()),
        }),
    );

    let server_config = ServerConfig {
        server_url: "http://localhost:8080".to_string(),
        agent_provider: AgentProvider {
            organization: "Test".to_string(),
            url: "https://test.com".to_string(),
        },
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        security_schemes,
        security: vec![{
            let mut req = HashMap::new();
            req.insert("apiKey".to_string(), vec![]);
            req
        }],
        capabilities: Default::default(),
        preferred_transport: Some("JSONRPC".to_string()),
        documentation_url: None,
    };

    let security_middleware = create_security_middleware(&server_config);

    let app = test::init_service(
        App::new()
            .wrap(security_middleware)
            .route(
                "/api/v1/agents",
                web::get().to(|ctx: SecurityContext| async move {
                    HttpResponse::Ok().json(format!(
                        "Authenticated as: {:?} with scheme: {}",
                        ctx.user_id, ctx.scheme_name
                    ))
                }),
            )
            .route("/health", web::get().to(|| async { HttpResponse::Ok().body("OK") })),
    )
    .await;

    // Test without API key - should fail
    let req = test::TestRequest::get().uri("/api/v1/agents").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401);

    // Test with API key - should succeed
    let req = test::TestRequest::get()
        .uri("/api/v1/agents")
        .insert_header(("X-API-Key", "test-key-123"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    // Test unprotected endpoint - should work without authentication
    let req = test::TestRequest::get().uri("/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn test_no_security_configured() {
    // Server config with no security schemes
    let server_config = ServerConfig {
        server_url: "http://localhost:8080".to_string(),
        agent_provider: AgentProvider {
            organization: "Test".to_string(),
            url: "https://test.com".to_string(),
        },
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        security_schemes: HashMap::new(),
        security: vec![],
        capabilities: Default::default(),
        preferred_transport: Some("JSONRPC".to_string()),
        documentation_url: None,
    };

    let security_middleware = create_security_middleware(&server_config);

    let app = test::init_service(
        App::new()
            .wrap(security_middleware)
            .route(
                "/api/v1/agents",
                web::get().to(|| async { HttpResponse::Ok().body("No security required") }),
            ),
    )
    .await;

    // When no security is configured, all requests should be allowed
    let req = test::TestRequest::get().uri("/api/v1/agents").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}