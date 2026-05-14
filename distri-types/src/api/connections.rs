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

use crate::connections::{AuthScope, Connection, ConnectionAuth, ConnectionKind};

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

/// Request body for `POST /v1/connections`.
///
/// `auth` carries the authentication material to attach to the new
/// connection (OAuth provider + scopes, Custom field schema, DistriNative,
/// or None).
///
/// `DistriNative` is reserved for the platform-seeded `distri` connection
/// and is rejected when supplied inline. `auth_scope=public` is rejected;
/// public scope belongs to channels, not connections.
///
/// `skill_content` is required for non-MCP custom connections; built-in
/// OAuth providers and MCP-kind rows fall back to bundled templates or the
/// MCP tool list respectively.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateConnectionRequest {
    pub name: String,
    pub auth_scope: AuthScope,
    /// Authentication material for this connection.
    pub auth: ConnectionAuth,
    /// Secret values for Workspace-scope Custom auth, keyed by field key.
    /// Ignored when `auth_scope=user`.
    #[serde(default)]
    pub secrets: HashMap<String, String>,
    /// **BYOK** (Bring-Your-Own OAuth client): workspace admin's own
    /// OAuth `client_id` to use instead of distri's platform-managed app.
    /// When present alongside `auth = Oauth { ... }`, distri stores
    /// these as workspace secrets under `connection.<id>.oauth_client_id`
    /// / `_secret` and uses them for the OAuth flow + refresh in place of
    /// the values from `connection_providers`. Same storage slot the MCP
    /// Dynamic Client Registration flow (RFC 7591) writes into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    /// Companion to `oauth_client_id`. Optional — public OAuth clients
    /// (`token_endpoint_auth_method=none`) don't require a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret: Option<String>,
    /// Markdown skill content. Required for custom non-MCP connections.
    #[serde(default)]
    pub skill_content: Option<String>,
    /// Capability surface — auth-only (`Default`) or MCP tool source (`Mcp`).
    #[serde(default)]
    pub kind: ConnectionKind,
}

/// PATCH body for updating a connection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateConnectionRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// Replace the embedded auth shape (e.g. edit Custom fields list).
    #[serde(default)]
    pub auth: Option<ConnectionAuth>,
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
