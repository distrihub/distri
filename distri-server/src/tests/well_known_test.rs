use actix_web::{test, web, App};
use distri::{
    agent::{AgentExecutor, AgentExecutorBuilder},
    types::{AgentDefinition, ModelSettings, ServerConfig, StoreConfig},
    HashMapTaskStore,
};
use distri_a2a::AgentCard;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::routes;

// Helper function to create test executor with agents
async fn create_test_executor() -> Arc<AgentExecutor> {
    let stores = StoreConfig::default().initialize().await.unwrap();
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()
        .unwrap();
    let executor = Arc::new(executor);

    // Register test agents
    let agent1 = AgentDefinition {
        name: "test-agent-1".to_string(),
        description: "A test agent for A2A discovery".to_string(),
        system_prompt: "You are a helpful test agent".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: Some("https://example.com/agent1.png".to_string()),
        max_iterations: Some(5),
        ..Default::default()
    };

    let agent2 = AgentDefinition {
        name: "test-agent-2".to_string(),
        description: "Another test agent for A2A discovery".to_string(),
        system_prompt: "You are another helpful test agent".to_string(),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        plan: None,
        icon_url: Some("https://example.com/agent2.png".to_string()),
        max_iterations: Some(5),
        sub_agents: vec![],
        ..Default::default()
    };

    executor.register_agent_definition(agent1).await.unwrap();
    executor.register_agent_definition(agent2).await.unwrap();

    executor
}

fn create_test_server_config() -> ServerConfig {
    ServerConfig {
        server_url: "http://localhost:8080".to_string(),
        agent_provider: distri_a2a::AgentProvider {
            organization: "Distri Test".to_string(),
            url: "https://github.com/distrihub/distri".to_string(),
        },
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
            .configure(routes::all),
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
            .configure(routes::all),
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
