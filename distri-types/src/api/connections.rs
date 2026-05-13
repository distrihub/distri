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

use crate::connections::{AuthScope, Connection, ConnectionKind};
use crate::credentials::CredentialMaterial;

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

/// How a `POST /v1/connections` body references the auth material.
///
/// `Existing` reuses a previously-created Credential (the path used by the
/// Connection Detail page and the Bot wizard once it's surfaced).
///
/// `Inline` lets the New Connection dialog create a Credential and a
/// Connection in one transaction so the user doesn't have to pre-create the
/// Credential row.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialRef {
    Existing {
        credential_id: Uuid,
    },
    Inline {
        material: CredentialMaterial,
        #[serde(default)]
        secrets: HashMap<String, String>,
    },
}

/// Unified request body for creating a connection.
///
/// One shape covers all auth types. `DistriNative` is not accepted inline —
/// that variant is reserved for the platform-seeded `distri` connection and
/// rejected at handler level. Use `CredentialRef::Existing` with the
/// system-seeded distri Credential instead.
///
/// `skill_content` is **required** for custom connections (Bearer, ApiKey, or
/// OAuth with a non-built-in provider). Built-in OAuth providers (google,
/// github, notion, slack, twitter, microsoft) fall back to the bundled skill
/// template when `skill_content` is omitted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateConnectionRequest {
    pub name: String,
    pub auth_scope: AuthScope,
    /// How to obtain the auth material. Either reuse an existing
    /// `Credential` or create one inline.
    pub credential: CredentialRef,
    /// Markdown skill content. Required for custom connections; optional for
    /// built-in OAuth providers that ship a template.
    #[serde(default)]
    pub skill_content: Option<String>,
    /// What surface this connection exposes. `Default` (the default) makes
    /// the connection auth-only; `Mcp { ... }` additionally makes it a
    /// remote MCP tool source whose tools are exposed to agents that
    /// reference its `name` in `ToolsConfig.mcp[].server`.
    #[serde(default)]
    pub kind: ConnectionKind,
}

/// PATCH body for updating a connection. Editing the linked Credential is
/// done via `PATCH /v1/credentials/{id}` directly.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateConnectionRequest {
    #[serde(default)]
    pub name: Option<String>,
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
