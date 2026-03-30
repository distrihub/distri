//! Typed request/response structs for the HTTP request proxy.
//!
//! Used by the `POST /request` server endpoint and the client-side
//! `http_request` tool handler. Serializes cleanly across the wire.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// HTTP method.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    #[serde(alias = "get")]
    GET,
    #[serde(alias = "post")]
    POST,
    #[serde(alias = "put")]
    PUT,
    #[serde(alias = "patch")]
    PATCH,
    #[serde(alias = "delete")]
    DELETE,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GET => write!(f, "GET"),
            Self::POST => write!(f, "POST"),
            Self::PUT => write!(f, "PUT"),
            Self::PATCH => write!(f, "PATCH"),
            Self::DELETE => write!(f, "DELETE"),
        }
    }
}

/// Input for an HTTP request — matches the tool parameter schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequestInput {
    /// Absolute URL. May contain `$VAR_NAME` for variable substitution.
    pub url: String,
    /// HTTP method.
    #[serde(default)]
    pub method: HttpMethod,
    /// Request headers. May contain `$VAR_NAME` for variable substitution.
    /// Set `x-connection-id` to inject an OAuth Bearer token.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body. Sent as JSON if no Content-Type is set.
    /// May contain `$VAR_NAME` for variable substitution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// Configuration for an HTTP request factory (type = "http").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpFactoryConfig {
    /// Base URL for all requests. May contain $VAR_NAME.
    pub base_url: String,
    /// Default headers merged into every request. May contain $VAR_NAME.
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// Input for a factory-created HTTP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFactoryToolInput {
    /// Request path (appended to base_url). Used for platform API calls.
    #[serde(default)]
    pub path: Option<String>,
    /// Absolute URL. When set, `path` is ignored and `base_url` is NOT prepended.
    /// Use this for external API calls (e.g., googleapis.com, slack.com).
    /// Set `x-connection-id` header to auto-inject OAuth Bearer token.
    #[serde(default)]
    pub url: Option<String>,
    /// HTTP method. Defaults to GET.
    #[serde(default)]
    pub method: HttpMethod,
    /// Additional headers (merged with factory defaults, per-call wins).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Request body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl HttpFactoryConfig {
    /// Build an HttpRequestInput from factory defaults + per-call input.
    ///
    /// If `url` is set, use it as-is (external API call — base_url is NOT prepended).
    /// If `path` is set, prepend base_url (platform API call).
    /// Factory default headers are merged, but per-call headers win on conflict.
    pub fn build_request(&self, input: &HttpFactoryToolInput) -> HttpRequestInput {
        let url = if let Some(ref absolute_url) = input.url {
            // External API call — use absolute URL directly, skip base_url
            absolute_url.clone()
        } else if let Some(ref path) = input.path {
            // Platform API call — prepend base_url
            format!("{}{}", self.base_url.trim_end_matches('/'), path)
        } else {
            // Fallback: just use base_url (shouldn't normally happen)
            self.base_url.clone()
        };

        // For external URLs (url field), don't inject factory default headers
        // (they contain platform auth like x-api-key which shouldn't leak to external APIs)
        let headers = if input.url.is_some() {
            input.headers.clone()
        } else {
            let mut headers = self.headers.clone();
            headers.extend(input.headers.clone());
            headers
        };

        HttpRequestInput {
            url,
            method: input.method.clone(),
            headers,
            body: input.body.clone(),
        }
    }
}

/// Response from an HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequestResponse {
    /// HTTP status code.
    pub status: u16,
    /// Whether the status is 2xx.
    pub ok: bool,
    /// Filtered response headers (content-type, location, ratelimit, etc.)
    pub headers: HashMap<String, String>,
    /// Response body — parsed as JSON if content-type is application/json,
    /// otherwise a string.
    pub body: serde_json::Value,
}
