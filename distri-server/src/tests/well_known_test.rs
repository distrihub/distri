use actix_web::{test, web, App};
use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    store::InMemoryAgentStore,
    types::{AgentDefinition, ModelSettings, ServerConfig},
    HashMapTaskStore,
};
use distri_a2a::AgentCard;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::routes;

// Helper function to create test executor with agents
async fn create_test_executor() -> Arc<AgentExecutor> {
    let agent_store = Arc::new(InMemoryAgentStore::new());
    let registry = Arc::new(tokio::sync::RwLock::new(
        distri::servers::registry::McpServerRegistry::new(),
    ));
    let builder = AgentExecutorBuilder::new()
        .with_agent_store(agent_store.clone())
        .with_registry(registry);

    let executor = Arc::new(builder.build().unwrap());

    // Register test agents
    let agent1 = AgentDefinition {
        name: "test-agent-1".to_string(),
        description: "A test agent for A2A discovery".to_string(),
        system_prompt: Some("You are a helpful test agent".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        parameters: None,
        response_format: None,
        history_size: Some(10),
        plan: None,
        icon_url: Some("https://example.com/agent1.png".to_string()),
        max_iterations: Some(5),
    };

    let agent2 = AgentDefinition {
        name: "test-agent-2".to_string(),
        description: "Another test agent for A2A discovery".to_string(),
        system_prompt: Some("You are another helpful test agent".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        parameters: None,
        response_format: None,
        history_size: Some(10),
        plan: None,
        icon_url: Some("https://example.com/agent2.png".to_string()),
        max_iterations: Some(5),
    };

    executor.register_default_agent(agent1).await.unwrap();
    executor.register_default_agent(agent2).await.unwrap();

    executor
}

fn create_test_server_config() -> ServerConfig {
    ServerConfig {
        server_url: "http://localhost:8080".to_string(),
        provider: Some(distri_a2a::AgentProvider {
            organization: "Distri Test".to_string(),
            url: "https://github.com/distrihub/distri".to_string(),
        }),
        default_input_modes: vec!["text/plain".to_string(), "text/markdown".to_string()],
        default_output_modes: vec!["text/plain".to_string(), "text/markdown".to_string()],
        security_schemes: std::collections::HashMap::new(),
        security: vec![],
        capabilities: distri_a2a::AgentCapabilities {
            streaming: true,
            push_notifications: true,
            state_transition_history: true,
            extensions: vec![],
        },
        preferred_transport: Some("JSONRPC".to_string()),
        documentation_url: Some("https://github.com/distrihub/distri/docs".to_string()),
    }
}

#[actix_web::test]
async fn test_agent_json_endpoint() {
    let executor = create_test_executor().await;
    let server_config = create_test_server_config();
    let task_store = Arc::new(HashMapTaskStore::new());
    let (event_broadcaster, _) = broadcast::channel::<String>(1000);

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(executor.clone()))
            .app_data(web::Data::new(executor.agent_store.clone()))
            .app_data(web::Data::new(task_store))
            .app_data(web::Data::new(event_broadcaster))
            .app_data(web::Data::new(server_config))
            .configure(routes::config),
    )
    .await;

    // Test specific agent via /agents/{agent_name}.json
    let req = test::TestRequest::get()
        .uri("/agents/test-agent-1.json")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: AgentCard = test::read_body_json(resp).await;
    assert_eq!(body.name, "test-agent-1");
    assert_eq!(body.description, "A test agent for A2A discovery");
    assert_eq!(body.version, distri_a2a::A2A_VERSION);
    assert!(body.url.contains("/api/v1/agents/test-agent-1"));
    assert_eq!(
        body.icon_url,
        Some("https://example.com/agent1.png".to_string())
    );
    assert_eq!(
        body.default_input_modes,
        vec!["text/plain", "text/markdown"]
    );
    assert_eq!(
        body.default_output_modes,
        vec!["text/plain", "text/markdown"]
    );
    assert!(body.capabilities.streaming);
    assert!(body.capabilities.push_notifications);

    // Test second agent
    let req = test::TestRequest::get()
        .uri("/agents/test-agent-2.json")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: AgentCard = test::read_body_json(resp).await;
    assert_eq!(body.name, "test-agent-2");
    assert_eq!(body.description, "Another test agent for A2A discovery");
    assert!(body.url.contains("/api/v1/agents/test-agent-2"));

    // Test non-existent agent
    let req = test::TestRequest::get()
        .uri("/agents/non-existent.json")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn test_well_known_a2a_info() {
    let executor = create_test_executor().await;
    let server_config = create_test_server_config();
    let task_store = Arc::new(HashMapTaskStore::new());
    let (event_broadcaster, _) = broadcast::channel::<String>(1000);

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(executor.clone()))
            .app_data(web::Data::new(executor.agent_store.clone()))
            .app_data(web::Data::new(task_store))
            .app_data(web::Data::new(event_broadcaster))
            .app_data(web::Data::new(server_config))
            .configure(routes::config),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/.well-known/a2a")
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: serde_json::Value = test::read_body_json(resp).await;

    // Verify discovery info structure
    assert_eq!(body["a2a_version"], distri_a2a::A2A_VERSION);
    assert_eq!(body["server"], "Distri");
    assert_eq!(body["transport"], "JSONRPC");

    // Verify endpoints are present
    let endpoints = &body["endpoints"];
    assert!(endpoints["agents"]
        .as_str()
        .unwrap()
        .contains("/api/v1/agents"));
    assert!(endpoints["agent_json"]
        .as_str()
        .unwrap()
        .contains("/agents/{agent_name}.json"));

    // Verify agents array
    let agents = body["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2);

    // Verify capabilities
    let capabilities = &body["capabilities"];
    assert_eq!(capabilities["streaming"], true);
    assert_eq!(capabilities["pushNotifications"], true);
}

#[actix_web::test]
async fn test_base_url_extraction() {
    let executor = create_test_executor().await;
    let server_config = create_test_server_config();
    let task_store = Arc::new(HashMapTaskStore::new());
    let (event_broadcaster, _) = broadcast::channel::<String>(1000);

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(executor.clone()))
            .app_data(web::Data::new(executor.agent_store.clone()))
            .app_data(web::Data::new(task_store))
            .app_data(web::Data::new(event_broadcaster))
            .app_data(web::Data::new(server_config))
            .configure(routes::config),
    )
    .await;

    // Test with custom host header using the new agent JSON endpoint
    let req = test::TestRequest::get()
        .uri("/agents/test-agent-1.json")
        .insert_header(("host", "example.com:8080"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body: AgentCard = test::read_body_json(resp).await;
    // URL should include the host from the request
    assert!(body.url.contains("example.com:8080"));
}
