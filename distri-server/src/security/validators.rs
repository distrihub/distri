use actix_web::{dev::ServiceRequest, error::ErrorUnauthorized, Error};
use distri_a2a::{APIKeySecurityScheme, HTTPAuthSecurityScheme};
use std::collections::HashMap;

use super::SecurityContext;

/// Validate API Key authentication
pub async fn validate_api_key(
    req: &ServiceRequest,
    scheme_name: &str,
    api_key_scheme: &APIKeySecurityScheme,
) -> Result<SecurityContext, Error> {
    let key_value = match api_key_scheme.location.as_str() {
        "header" => req
            .headers()
            .get(&api_key_scheme.name)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string()),
        "query" => req
            .query_string()
            .split('&')
            .find_map(|param| {
                let mut parts = param.split('=');
                if parts.next() == Some(&api_key_scheme.name) {
                    parts.next().map(|s| s.to_string())
                } else {
                    None
                }
            }),
        "cookie" => {
            // Cookie extraction would be implemented here
            None
        }
        _ => None,
    };

    match key_value {
        Some(key) => {
            // In a real implementation, you'd validate the key against a database
            // For now, we'll accept any non-empty key as valid
            if !key.is_empty() {
                Ok(SecurityContext::new(
                    Some(format!("api_key_user_{}", key.chars().take(8).collect::<String>())),
                    vec!["read".to_string(), "write".to_string()],
                    scheme_name.to_string(),
                ))
            } else {
                Err(ErrorUnauthorized("Invalid API key"))
            }
        }
        None => Err(ErrorUnauthorized("API key required")),
    }
}

/// Validate HTTP authentication (Bearer token, Basic auth, etc.)
pub async fn validate_http_auth(
    req: &ServiceRequest,
    scheme_name: &str,
    http_scheme: &HTTPAuthSecurityScheme,
) -> Result<SecurityContext, Error> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth_value) => {
            if auth_value.starts_with(&format!("{} ", http_scheme.scheme)) {
                let token = auth_value
                    .strip_prefix(&format!("{} ", http_scheme.scheme))
                    .unwrap_or("");

                if !token.is_empty() {
                    // In a real implementation, you'd validate the token
                    // For Bearer tokens, this might involve JWT validation
                    // For Basic auth, this would involve credential validation
                    Ok(SecurityContext::new(
                        Some(format!("http_user_{}", token.chars().take(8).collect::<String>())),
                        vec!["read".to_string(), "write".to_string()],
                        scheme_name.to_string(),
                    ))
                } else {
                    Err(ErrorUnauthorized("Invalid token"))
                }
            } else {
                Err(ErrorUnauthorized("Invalid authentication scheme"))
            }
        }
        None => Err(ErrorUnauthorized("Authorization header required")),
    }
}

/// Default API key validator - checks against a static set of valid keys
pub fn create_default_api_key_validator() -> Box<dyn Fn(&str) -> bool + Send + Sync> {
    // In a real implementation, this would check against a database
    let valid_keys = vec!["test-key-123", "admin-key-456", "user-key-789"];
    Box::new(move |key: &str| valid_keys.contains(&key))
}

/// Default Bearer token validator - checks JWT or other token format
pub fn create_default_bearer_validator() -> Box<dyn Fn(&str) -> Result<HashMap<String, String>, String> + Send + Sync> {
    // In a real implementation, this would validate JWT tokens
    Box::new(move |token: &str| {
        if token.starts_with("valid-") {
            let mut claims = HashMap::new();
            claims.insert("sub".to_string(), token.to_string());
            claims.insert("scope".to_string(), "read write".to_string());
            Ok(claims)
        } else {
            Err("Invalid token".to_string())
        }
    })
}