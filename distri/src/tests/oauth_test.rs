use distri::{
    oauth::{OAuthConfig, OAuthManager, OAuthService},
    stores::{AuthStore, InMemoryAuthStore, OAuthTokens},
    types::McpSession,
};
use std::sync::Arc;

#[tokio::test]
async fn test_oauth_manager_creation() {
    let oauth_manager = OAuthManager::new();
    assert!(oauth_manager.get_service("nonexistent").is_none());
}

#[tokio::test]
async fn test_oauth_service_registration() {
    let mut oauth_manager = OAuthManager::new();
    
    let config = OAuthConfig {
        client_id: "test_client_id".to_string(),
        client_secret: "test_client_secret".to_string(),
        authorization_url: "https://test.com/oauth/authorize".to_string(),
        token_url: "https://test.com/oauth/token".to_string(),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    };
    
    let service = OAuthService::new("test_service".to_string(), config);
    oauth_manager.register_service(service);
    
    assert!(oauth_manager.get_service("test_service").is_some());
    assert!(oauth_manager.get_service("nonexistent").is_none());
}

#[tokio::test]
async fn test_oauth_authorization_url_generation() {
    let config = OAuthConfig {
        client_id: "test_client_id".to_string(),
        client_secret: "test_client_secret".to_string(),
        authorization_url: "https://test.com/oauth/authorize".to_string(),
        token_url: "https://test.com/oauth/token".to_string(),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    };
    
    let service = OAuthService::new("test_service".to_string(), config);
    let state = "test_state_123";
    let auth_url = service.get_authorization_url(state);
    
    assert!(auth_url.contains("client_id=test_client_id"));
    assert!(auth_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback"));
    assert!(auth_url.contains("response_type=code"));
    assert!(auth_url.contains("scope=read%20write"));
    assert!(auth_url.contains("state=test_state_123"));
}

#[tokio::test]
async fn test_auth_store_token_operations() {
    let auth_store = Arc::new(InMemoryAuthStore::new());
    
    // Test storing tokens
    auth_store
        .store_oauth_tokens(
            "test_service",
            "test_user",
            "access_token_123",
            Some("refresh_token_456"),
            Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .unwrap();
    
    // Test retrieving tokens
    let tokens = auth_store.get_oauth_tokens("test_service", "test_user").await.unwrap();
    assert!(tokens.is_some());
    
    let tokens = tokens.unwrap();
    assert_eq!(tokens.access_token, "access_token_123");
    assert_eq!(tokens.refresh_token, Some("refresh_token_456".to_string()));
    assert!(tokens.expires_at.is_some());
    
    // Test checking if tokens are valid
    let has_valid_tokens = auth_store.has_valid_oauth_tokens("test_service", "test_user").await.unwrap();
    assert!(has_valid_tokens);
    
    // Test removing tokens
    auth_store.remove_oauth_tokens("test_service", "test_user").await.unwrap();
    
    let tokens = auth_store.get_oauth_tokens("test_service", "test_user").await.unwrap();
    assert!(tokens.is_none());
}

#[tokio::test]
async fn test_oauth_state_operations() {
    let auth_store = Arc::new(InMemoryAuthStore::new());
    
    // Test storing OAuth state
    auth_store
        .store_oauth_state(
            "test_state_123",
            "test_service",
            "test_user",
            "http://localhost:8080/callback",
        )
        .await
        .unwrap();
    
    // Test retrieving OAuth state
    let state = auth_store.get_oauth_state("test_state_123").await.unwrap();
    assert!(state.is_some());
    
    let state = state.unwrap();
    assert_eq!(state.service_name, "test_service");
    assert_eq!(state.user_id, "test_user");
    assert_eq!(state.redirect_uri, "http://localhost:8080/callback");
    
    // Test removing OAuth state
    auth_store.remove_oauth_state("test_state_123").await.unwrap();
    
    let state = auth_store.get_oauth_state("test_state_123").await.unwrap();
    assert!(state.is_none());
}

#[tokio::test]
async fn test_session_creation_from_tokens() {
    let auth_store = Arc::new(InMemoryAuthStore::new());
    let oauth_manager = OAuthManager::new();
    
    // Store tokens first
    auth_store
        .store_oauth_tokens(
            "test_service",
            "test_user",
            "access_token_123",
            Some("refresh_token_456"),
            Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .unwrap();
    
    // Create session from tokens
    let session = oauth_manager
        .create_session_from_tokens(auth_store.as_ref(), "test_service", "test_user")
        .await
        .unwrap();
    
    assert!(session.is_some());
    
    let session = session.unwrap();
    assert_eq!(session.token, "oauth_test_service");
    assert_eq!(session.oauth_access_token, Some("access_token_123".to_string()));
    assert_eq!(session.oauth_refresh_token, Some("refresh_token_456".to_string()));
    assert!(session.oauth_expires_at.is_some());
    assert_eq!(session.oauth_token_type, Some("Bearer".to_string()));
}

#[tokio::test]
async fn test_auth_required_check() {
    let auth_store = Arc::new(InMemoryAuthStore::new());
    let mut oauth_manager = OAuthManager::new();
    
    // Register a service
    let config = OAuthConfig {
        client_id: "test_client_id".to_string(),
        client_secret: "test_client_secret".to_string(),
        authorization_url: "https://test.com/oauth/authorize".to_string(),
        token_url: "https://test.com/oauth/token".to_string(),
        redirect_uri: "http://localhost:8080/callback".to_string(),
        scopes: vec!["read".to_string()],
    };
    
    let service = OAuthService::new("test_service".to_string(), config);
    oauth_manager.register_service(service);
    
    // Test auth required when no tokens exist
    let auth_required = oauth_manager
        .check_auth_required(auth_store.as_ref(), "test_service", "test_user")
        .await
        .unwrap();
    assert!(auth_required);
    
    // Store tokens
    auth_store
        .store_oauth_tokens(
            "test_service",
            "test_user",
            "access_token_123",
            None,
            Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        )
        .await
        .unwrap();
    
    // Test auth not required when tokens exist
    let auth_required = oauth_manager
        .check_auth_required(auth_store.as_ref(), "test_service", "test_user")
        .await
        .unwrap();
    assert!(!auth_required);
    
    // Test unknown service doesn't require auth
    let auth_required = oauth_manager
        .check_auth_required(auth_store.as_ref(), "unknown_service", "test_user")
        .await
        .unwrap();
    assert!(!auth_required);
}