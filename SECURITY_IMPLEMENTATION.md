# Security Implementation for Distri Server

This document explains the security implementation added to the distri-server based on the A2A (Agent-to-Agent) specification security schemes.

## Overview

The security system provides authentication and authorization for API endpoints at the distri-server level. It supports multiple security schemes as defined in the A2A specification:

- **API Key authentication** - Header, query parameter, or cookie-based
- **HTTP authentication** - Bearer tokens, Basic auth, etc.
- **OAuth2** (framework ready, implementation needed)
- **OpenID Connect** (framework ready, implementation needed)

## Architecture

### Components

1. **SecurityMiddleware** - Actix Web middleware that intercepts requests and validates authentication
2. **SecurityContext** - Request context containing user information and permissions
3. **Validators** - Functions that validate different authentication schemes
4. **Configuration** - YAML-based configuration for security schemes

### Request Flow

1. Request arrives at the server
2. SecurityMiddleware checks if the endpoint requires authentication
3. If authentication is required, it validates using configured security schemes
4. If valid, SecurityContext is added to the request
5. Request continues to the handler
6. If invalid, 401 Unauthorized is returned

## Configuration

### Basic Security Configuration

```yaml
server:
  # Security schemes define HOW authentication works
  security_schemes:
    # API Key in header
    apiKey:
      type: "apiKey"
      name: "X-API-Key"
      in: "header"
      description: "API key for authentication"
    
    # Bearer token authentication
    bearerAuth:
      type: "http"
      scheme: "bearer"
      bearer_format: "JWT"
      description: "Bearer token authentication"
    
    # Basic authentication
    basicAuth:
      type: "http"
      scheme: "basic"
      description: "Basic HTTP authentication"

  # Security requirements define WHICH schemes are required
  security:
    # Option 1: Require API key OR bearer token
    - apiKey: []
      bearerAuth: []
    
    # Option 2: Require specific OAuth2 scopes
    # - oauth2: ["read", "write"]
```

### Complete Example Configuration

See `security-config-example.yaml` for a complete configuration example.

## Protected Endpoints

By default, these endpoint patterns require authentication:
- `/api/v1/agents/**`
- `/api/v1/tasks/**`
- `/api/v1/threads/**`

Public endpoints (no authentication required):
- `/health`
- `/`
- `/.well-known/**`

## Usage Examples

### API Key Authentication

```bash
# Without API key - returns 401
curl http://localhost:8080/api/v1/agents

# With API key - returns 200
curl -H "X-API-Key: your-api-key-here" \
     http://localhost:8080/api/v1/agents
```

### Bearer Token Authentication

```bash
# With Bearer token
curl -H "Authorization: Bearer your-jwt-token-here" \
     http://localhost:8080/api/v1/agents
```

### Basic Authentication

```bash
# With Basic auth
curl -H "Authorization: Basic $(echo -n 'username:password' | base64)" \
     http://localhost:8080/api/v1/agents
```

## Implementation Details

### Adding New Security Schemes

To add support for a new security scheme:

1. **Add to A2A types** (if not already present in `distri-a2a`)
2. **Add validator function** in `distri-server/src/security/validators.rs`
3. **Update middleware** in `distri-server/src/security/mod.rs` to call the validator

Example validator function:

```rust
pub async fn validate_custom_auth(
    req: &ServiceRequest,
    scheme_name: &str,
    custom_scheme: &CustomSecurityScheme,
) -> Result<SecurityContext, Error> {
    // Implementation here
    // Return Ok(SecurityContext) if valid
    // Return Err(ErrorUnauthorized) if invalid
}
```

### Accessing Security Context in Handlers

```rust
use distri_server::security::SecurityContext;

async fn protected_handler(ctx: SecurityContext) -> HttpResponse {
    match ctx.user_id {
        Some(user_id) => {
            HttpResponse::Ok().json(format!("Hello, user: {}", user_id))
        }
        None => {
            HttpResponse::Ok().json("Anonymous access")
        }
    }
}
```

### Custom Validation Logic

The current implementation uses simple validation logic for demonstration:

- **API Keys**: Any non-empty key is considered valid
- **Bearer Tokens**: Any non-empty token is considered valid
- **Basic Auth**: Any non-empty credentials are considered valid

For production use, implement proper validation:

```rust
// In validators.rs
pub async fn validate_api_key(
    req: &ServiceRequest,
    scheme_name: &str,
    api_key_scheme: &APIKeySecurityScheme,
) -> Result<SecurityContext, Error> {
    let key_value = extract_api_key(req, api_key_scheme)?;
    
    // TODO: Replace with real validation
    if is_valid_api_key(&key_value).await {
        let user_info = get_user_for_api_key(&key_value).await?;
        Ok(SecurityContext::new(
            Some(user_info.user_id),
            user_info.scopes,
            scheme_name.to_string(),
        ))
    } else {
        Err(ErrorUnauthorized("Invalid API key"))
    }
}
```

## Testing

Run the security tests:

```bash
cd distri-server
cargo test security
```

## Agent Card Integration

When security schemes are configured, they are automatically included in the agent cards returned by the A2A discovery endpoints. This allows clients to understand what authentication is required.

The agent card will include:
- `securitySchemes` - Details of available authentication methods
- `security` - Requirements for which schemes must be used

## Future Enhancements

1. **OAuth2 Implementation** - Complete implementation for OAuth2 flows
2. **OpenID Connect** - Implementation for OIDC authentication
3. **Scope-based Authorization** - Fine-grained permissions based on scopes
4. **Rate Limiting** - Per-user or per-key rate limiting
5. **Audit Logging** - Security event logging
6. **Token Refresh** - Automatic token refresh for supported schemes

## Security Considerations

1. **HTTPS Only** - Always use HTTPS in production for token-based authentication
2. **Token Storage** - Store tokens securely, consider using secure headers
3. **Key Rotation** - Implement API key rotation policies
4. **Monitoring** - Monitor for authentication failures and suspicious activity
5. **Validation** - Implement proper token validation (JWT verification, etc.)

## Error Responses

Authentication failures return standard HTTP 401 responses:

```json
{
  "error": "Authentication required",
  "message": "API key required",
  "status": 401
}
```

## Integration with Standard Agents vs Custom Agents

The security middleware works at the server level, so it applies to:

- **Standard agents** - Registered through the agent store
- **Custom agents** - Any custom implementations
- **All A2A endpoints** - Message sending, task management, etc.

The security context is available in all handler functions, allowing both standard and custom agents to access authentication information.