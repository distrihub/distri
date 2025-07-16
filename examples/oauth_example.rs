use distri::{
    agent::AgentExecutorBuilder,
    oauth::{OAuthConfig, OAuthManager, OAuthService},
    stores::InitializedStores,
    types::{AgentDefinition, Configuration, ModelSettings},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create OAuth manager and register services
    let mut oauth_manager = OAuthManager::new();

    // Register Reddit OAuth service
    let reddit_config = OAuthConfig {
        client_id: "your_reddit_client_id".to_string(),
        client_secret: "your_reddit_client_secret".to_string(),
        authorization_url: "https://www.reddit.com/api/v1/authorize".to_string(),
        token_url: "https://www.reddit.com/api/v1/access_token".to_string(),
        redirect_uri: "http://localhost:8080/api/v1/oauth/callback".to_string(),
        scopes: vec!["read".to_string(), "history".to_string(), "identity".to_string()],
    };

    let reddit_service = OAuthService::new("reddit".to_string(), reddit_config);
    oauth_manager.register_service(reddit_service);

    // Register Google OAuth service
    let google_config = OAuthConfig {
        client_id: "your_google_client_id".to_string(),
        client_secret: "your_google_client_secret".to_string(),
        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        redirect_uri: "http://localhost:8080/api/v1/oauth/callback".to_string(),
        scopes: vec![
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
        ],
    };

    let google_service = OAuthService::new("google".to_string(), google_config);
    oauth_manager.register_service(google_service);

    // Create configuration
    let config = Configuration {
        agents: vec![
            AgentDefinition {
                name: "oauth-assistant".to_string(),
                description: "Assistant with OAuth-enabled tools".to_string(),
                system_prompt: "You are a helpful assistant that can access Reddit and Google services. When a user wants to use Reddit tools, you'll need to guide them through OAuth authentication first.".to_string(),
                mcp_servers: vec![
                    distri::types::McpDefinition {
                        name: "reddit".to_string(),
                        filter: None,
                        r#type: distri::types::McpServerType::Tool,
                    },
                    distri::types::McpDefinition {
                        name: "google".to_string(),
                        filter: None,
                        r#type: distri::types::McpServerType::Tool,
                    },
                ],
                model_settings: ModelSettings::default(),
                history_size: Some(10),
                include_tools: true,
                ..Default::default()
            },
        ],
        sessions: std::collections::HashMap::new(),
        mcp_servers: vec![],
        server: None,
        stores: None,
    };

    // Initialize stores
    let stores = config
        .stores
        .clone()
        .unwrap_or_default()
        .initialize()
        .await?;

    // Create agent executor with OAuth support
    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_oauth_manager(Arc::new(oauth_manager))
        .build()?;

    let executor = Arc::new(executor);

    // Register agents from configuration
    for definition in &config.agents {
        executor
            .register_agent_definition(definition.clone())
            .await?;
    }

    println!("OAuth-enabled Distri server initialized successfully!");
    println!("Available OAuth services: reddit, google");
    println!("OAuth endpoints:");
    println!("  POST /api/v1/oauth/initiate - Start OAuth flow");
    println!("  POST /api/v1/oauth/callback - Handle OAuth callback");
    println!();
    println!("Example usage:");
    println!("1. User tries to use Reddit tool");
    println!("2. System returns AuthRequired error");
    println!("3. Client calls /api/v1/oauth/initiate");
    println!("4. User authorizes application");
    println!("5. OAuth callback stores tokens");
    println!("6. User can now use Reddit tools");

    // Keep the server running
    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");

    Ok(())
}