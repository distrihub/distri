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

use crate::connections::{
    AuthScope, Connection, ConnectionAuth, ConnectionKind, OAuthProviderConfig,
};

// Re-export the inline provider config from `crate::connections` so the
// API module is a one-stop import for handlers.
pub use crate::connections::OAuthProviderConfig as ApiOAuthProviderConfig;

/// Stored in `Connection.config` to carry provider-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ConnectionConfig {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub secret_keys: Vec<String>,
    /// Workspace-scope `extra_auth_params` (e.g. Slack `team`) the admin
    /// supplied at create / reconnect time. Persisted so the Edit dialog
    /// can pre-populate the schema-driven inputs — otherwise the admin
    /// has to remember and re-type the value every time.
    ///
    /// User-scope connections do NOT use this field — per-user extras
    /// come from each end-user via the configure flow and live only as
    /// transient inputs to the per-user auth URL.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extra_auth_params: std::collections::HashMap<String, String>,
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

/// Request body for `POST /v1/connections/oauth/discover` — given an MCP
/// server URL, returns the OAuth authorization-server metadata that
/// protects it (RFC 8414 + RFC 9728). Used by the UI's "OAuth (discover)"
/// custom-MCP create path. Pure read — no connection row is created.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DiscoverOAuthRequest {
    /// MCP server URL (e.g. `https://mcp.slack.com/mcp`).
    pub url: String,
}

/// Response body for `POST /v1/connections/{id}/test` — runs a one-shot
/// MCP probe using the connection's resolved auth headers (Bearer token
/// from Redis for OAuth, custom field values for Custom auth).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpProbeResponse {
    Ok {
        tool_count: usize,
        tool_names: Vec<String>,
    },
    Error {
        message: String,
    },
}

/// Response body for `POST /v1/connections/oauth/discover`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DiscoverOAuthResponse {
    /// Full provider declaration built from the discovered RFC 8414
    /// metadata. Forwarded verbatim into `auth.provider` on the
    /// create body. UI may pre-populate `name` from the URL host, and
    /// `display_name` is left for the admin to set.
    pub provider_config: OAuthProviderConfig,
    /// Scopes advertised by the auth server — UI pre-fills its scope picker.
    pub suggested_scopes: Vec<String>,
    /// True when `provider_config.registration_endpoint` is set. UI uses
    /// this to surface DCR as an option (rarely exercised in practice).
    pub supports_dcr: bool,
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

/// PATCH body for updating a connection. All fields are optional; only
/// supplied fields are applied. Powers the create-dialog "upsert" flow
/// where the row is created on first submit and refined (name, auth) on
/// subsequent resubmits before the OAuth round-trip completes.
///
/// `auth_scope` is intentionally **not** patchable — once a row exists,
/// its scope is frozen because flipping it would re-key per-user vs
/// workspace token storage and silently strand existing tokens. The UI
/// locks the scope toggle once the row is created.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateConnectionRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// Replace the embedded auth shape (e.g. edit Custom fields list,
    /// rotate OAuth scopes).
    #[serde(default)]
    pub auth: Option<ConnectionAuth>,
}

/// Response body for `POST /v1/connections`.
///
/// Pure CRUD — returns the persisted Connection row regardless of auth type.
/// For OAuth connections, the row starts in `pending` status; the UI follows
/// up with `POST /v1/connections/{id}/authorize` to obtain the consent-screen
/// URL and complete the flow.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateConnectionResponse {
    pub connection: Connection,
}

/// Request body for `POST /v1/connections/{id}/authorize` — trigger an OAuth
/// consent-screen redirect for an existing connection. Same endpoint covers
/// both first-time authorization and reconnect (token refresh in place).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct AuthorizeConnectionRequest {
    /// Provider-agnostic passthrough — appended verbatim to the OAuth URL.
    /// Validated server-side against the provider's `auth_params_schema`
    /// (surfaced on `GET /v1/connections/providers`). Examples: Slack
    /// `{"team":"T0123ABCD"}`, Microsoft `{"tenant":"..."}`,
    /// Google `{"login_hint":"..."}`.
    #[serde(default)]
    pub extra_auth_params: HashMap<String, String>,
    /// OAuth provider redirect URL. Caller (UI) should pass
    /// `<window.location.origin>/auth/callback` so the consent flow
    /// returns to whatever domain the user is currently on (localhost
    /// dev vs prod app domain). The URL must be registered with the
    /// OAuth provider; the server uses what the caller provides
    /// verbatim. Falls back to the server's static `WEB_APP_URL` if
    /// absent — for legacy / non-browser callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
}

/// Response body for `POST /v1/connections/{id}/authorize`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct AuthorizeConnectionResponse {
    pub connection_id: Uuid,
    /// Raw provider auth URL (accounts.google.com/...). Agents should
    /// prefer `setup_url` when sending a link to end users.
    pub auth_url: String,
    /// Distri-hosted redirect URL. Share this with end users instead of
    /// the raw provider URL — it 302-redirects to `auth_url`, but the
    /// visible domain is the distri cloud, which is friendlier in chat
    /// messages and survives link previews.
    pub setup_url: String,
}
