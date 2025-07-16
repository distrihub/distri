# OAuth Authentication in Distri

This document explains how to implement and use OAuth authentication in Distri for accessing external services like Reddit, Google, and other OAuth-enabled APIs.

## Overview

Distri now supports OAuth 2.0 authentication flows, allowing agents to securely access external services that require user authentication. The implementation includes:

- **AuthStore**: Manages OAuth tokens and state
- **OAuthManager**: Handles OAuth service registration and flow management
- **OAuth Flow Handlers**: Manages authorization code flow
- **Mock Reddit MCP Server**: Example implementation requiring OAuth

## Architecture

### Components

1. **AuthStore Trait**: Defines the interface for storing and retrieving OAuth tokens
2. **InMemoryAuthStore**: In-memory implementation of AuthStore
3. **OAuthManager**: Manages OAuth services and handles authentication flows
4. **OAuthHandler**: HTTP handlers for OAuth endpoints
5. **RedditMcpServer**: Example MCP server requiring OAuth authentication

### Data Structures

- `OAuthTokens`: Stores access tokens, refresh tokens, and metadata
- `OAuthState`: Manages OAuth state for callback verification
- `McpSession`: Extended to include OAuth token information

## Setup Instructions

### 1. Configure OAuth Services

Create a configuration file with your OAuth service details:

```yaml
# oauth-config.yaml
oauth_services:
  reddit:
    client_id: "your_reddit_client_id"
    client_secret: "your_reddit_client_secret"
    authorization_url: "https://www.reddit.com/api/v1/authorize"
    token_url: "https://www.reddit.com/api/v1/access_token"
    redirect_uri: "http://localhost:8080/api/v1/oauth/callback"
    scopes:
      - "read"
      - "history"
      - "identity"
```

### 2. Register OAuth Services

In your application startup code:

```rust
use distri::oauth::{OAuthConfig, OAuthManager, OAuthService};

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
```

### 3. Initialize AgentExecutor with OAuth

```rust
use distri::agent::AgentExecutorBuilder;

let executor = AgentExecutorBuilder::default()
    .with_stores(stores)
    .with_oauth_manager(Arc::new(oauth_manager))
    .build()?;
```

## OAuth Flow

### 1. User Requests OAuth-Protected Tool

When a user tries to use a tool that requires OAuth (e.g., Reddit tools), the system checks if valid tokens exist.

### 2. Authentication Required Response

If no valid tokens are found, the agent returns an `AuthRequired` error with instructions:

```
OAuth authentication required for reddit service. Please authenticate first.
```

### 3. Initiate OAuth Flow

The user (or client application) calls the OAuth initiate endpoint:

```bash
curl -X POST http://localhost:8080/api/v1/oauth/initiate \
  -H "Content-Type: application/json" \
  -d '{
    "service_name": "reddit",
    "user_id": "user123",
    "redirect_uri": "http://localhost:3000/callback"
  }'
```

Response:
```json
{
  "authorization_url": "https://www.reddit.com/api/v1/authorize?client_id=...&redirect_uri=...&response_type=code&scope=read%20history%20identity&state=uuid-here",
  "state": "uuid-here"
}
```

### 4. User Authorization

The user visits the authorization URL and grants permissions to the application.

### 5. OAuth Callback

The OAuth provider redirects to the callback URL with an authorization code:

```bash
curl -X POST http://localhost:8080/api/v1/oauth/callback \
  -H "Content-Type: application/json" \
  -d '{
    "code": "authorization_code_here",
    "state": "uuid-here",
    "user_id": "user123"
  }'
```

### 6. Token Exchange and Storage

The system exchanges the authorization code for access tokens and stores them securely.

### 7. Tool Execution

Now the user can use Reddit tools with the stored OAuth tokens.

## API Endpoints

### POST /api/v1/oauth/initiate

Initiates OAuth flow for a service.

**Request:**
```json
{
  "service_name": "reddit",
  "user_id": "user123",
  "redirect_uri": "http://localhost:3000/callback"
}
```

**Response:**
```json
{
  "authorization_url": "https://www.reddit.com/api/v1/authorize?...",
  "state": "uuid-here"
}
```

### POST /api/v1/oauth/callback

Handles OAuth callback with authorization code.

**Request:**
```json
{
  "code": "authorization_code",
  "state": "uuid-here",
  "user_id": "user123"
}
```

**Response:**
```json
{
  "message": "OAuth authentication successful"
}
```

## Example: Reddit Integration

### 1. Create Reddit MCP Server

```rust
use distri::servers::reddit::RedditMcpServer;

let reddit_server = RedditMcpServer::new(auth_store.clone());
```

### 2. Register Reddit Tools

The Reddit MCP server provides these tools:
- `reddit_get_user_posts`: Get posts by a Reddit user
- `reddit_get_subreddit_posts`: Get posts from a subreddit
- `reddit_get_post_comments`: Get comments for a Reddit post

### 3. Use Reddit Tools

Once authenticated, users can use Reddit tools:

```
User: "Get my recent Reddit posts"
Agent: "I'll fetch your recent Reddit posts for you."
[Tool: reddit_get_user_posts with stored OAuth tokens]
```

## Security Considerations

1. **Token Storage**: OAuth tokens are stored securely in the AuthStore
2. **State Verification**: OAuth state is verified to prevent CSRF attacks
3. **Token Expiry**: Tokens are checked for expiry and refreshed when needed
4. **User Isolation**: Tokens are stored per user and service

## Error Handling

### AuthRequired Error

When OAuth authentication is required:

```rust
AgentError::AuthRequired("OAuth authentication required for reddit service. Please authenticate first.".to_string())
```

### Token Expiry

When tokens expire, the system can attempt to refresh them using refresh tokens.

## Extending for New Services

To add support for a new OAuth service:

1. **Create OAuth Configuration**:
```rust
let new_service_config = OAuthConfig {
    client_id: "your_client_id".to_string(),
    client_secret: "your_client_secret".to_string(),
    authorization_url: "https://service.com/oauth/authorize".to_string(),
    token_url: "https://service.com/oauth/token".to_string(),
    redirect_uri: "http://localhost:8080/api/v1/oauth/callback".to_string(),
    scopes: vec!["scope1".to_string(), "scope2".to_string()],
};
```

2. **Register the Service**:
```rust
let new_service = OAuthService::new("new_service".to_string(), new_service_config);
oauth_manager.register_service(new_service);
```

3. **Create MCP Server**:
Implement a new MCP server that requires OAuth authentication, similar to the RedditMcpServer example.

4. **Update Tool Mapping**:
Add the service name to the `get_oauth_service_for_tool` method in AgentExecutor.

## Testing

### Test OAuth Flow

1. Start the server with OAuth configuration
2. Try to use a Reddit tool without authentication
3. Follow the OAuth flow to authenticate
4. Verify the tool works after authentication

### Mock OAuth Provider

For testing, you can create a mock OAuth provider that simulates the OAuth flow without requiring real external services.

## Troubleshooting

### Common Issues

1. **Invalid Client ID/Secret**: Ensure OAuth credentials are correct
2. **Redirect URI Mismatch**: Verify redirect URI matches OAuth app configuration
3. **State Verification Failed**: Check that state parameter is properly handled
4. **Token Expiry**: Implement token refresh logic for expired tokens

### Debug Logging

Enable debug logging to trace OAuth flow:

```rust
tracing::debug!("OAuth flow initiated for service: {}", service_name);
tracing::debug!("Token exchange successful for user: {}", user_id);
```

## Future Enhancements

1. **Token Refresh**: Automatic token refresh using refresh tokens
2. **Multiple OAuth Providers**: Support for different OAuth providers (Google, GitHub, etc.)
3. **Persistent Storage**: Redis-based token storage for production use
4. **OAuth 1.0a Support**: Support for OAuth 1.0a services like Twitter
5. **PKCE Support**: Proof Key for Code Exchange for enhanced security