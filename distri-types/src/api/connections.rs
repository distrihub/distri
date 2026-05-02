//! Wire-level request/response DTOs for the connections API.
//!
//! These types are shared between distri-cloud and distri-server so both
//! services expose byte-identical JSON on the wire. Do not add server-specific
//! logic here — this module is pure serde shapes.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::connections::{AuthScope, AuthType, Connection};

/// Stored in `Connection.config` to carry provider-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ConnectionConfig {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub secret_keys: Vec<String>,
}

/// Response body for `GET /v1/connections/{id}/token`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scopes: Vec<String>,
}

/// Response body for `POST /v1/connections/oauth/callback`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct OAuthCallbackResponse {
    pub connected: bool,
    pub scopes: Vec<String>,
}

/// Request body for `POST /v1/connections/oauth/callback`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct OAuthCallbackRequest {
    pub code: String,
    pub state: String,
}

/// Unified request body for creating a connection.
///
/// One shape covers all auth types. `DistriNative` is not accepted — that
/// variant is reserved for the platform-seeded `distri` connection and rejected
/// at handler level.
///
/// `skill_content` is **required** for custom connections (Bearer, ApiKey, or
/// OAuth with a non-built-in provider). Built-in OAuth providers (google,
/// github, notion, slack, twitter, microsoft) fall back to the bundled skill
/// template when `skill_content` is omitted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateConnectionRequest {
    pub name: String,
    pub auth_scope: AuthScope,
    pub auth_type: AuthType,
    /// Optional secrets for Bearer/ApiKey. Keyed by the semantics of that
    /// auth_type — e.g. `{"value": "sk-..."}` for a Bearer/ApiKey value.
    #[serde(default)]
    pub secrets: HashMap<String, String>,
    /// Markdown skill content. Required for custom connections; optional for
    /// built-in OAuth providers that ship a template.
    #[serde(default)]
    pub skill_content: Option<String>,
}

/// PATCH body for updating a connection. Both fields optional; at least one
/// should be present. `auth_type` is only accepted when the existing
/// connection is of type `custom`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateConnectionRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub auth_type: Option<AuthType>,
}

/// Response body for `POST /v1/connections`.
///
/// Tagged union: the `type` field on the wire is either `"oauth_redirect"` or
/// `"connected"`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type")]
pub enum CreateConnectionResponse {
    /// The provider requires an OAuth consent-screen redirect.
    #[serde(rename = "oauth_redirect")]
    OAuthRedirect {
        connection_id: Uuid,
        /// Raw provider auth URL (accounts.google.com/...). Agents should
        /// prefer `setup_url` when sending a link to end users.
        auth_url: String,
        /// Distri-hosted redirect URL. Share this with end users instead of
        /// the raw provider URL — it 302-redirects to `auth_url`, but the
        /// visible domain is the distri cloud, which is friendlier in chat
        /// messages and survives link previews.
        setup_url: String,
    },
    /// The connection was established immediately (e.g. custom / API-key).
    #[serde(rename = "connected")]
    Connected { connection: Connection },
}
