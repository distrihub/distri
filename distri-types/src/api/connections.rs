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

/// Response body for `POST /v1/connections/oauth/discover`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DiscoverOAuthResponse {
    /// Authorization-server metadata (issuer, endpoints, optional
    /// registration endpoint). Forward verbatim into `auth.oauth_metadata`
    /// on the create body.
    pub metadata: crate::connections::OAuthMetadata,
    /// Scopes advertised by the auth server — UI pre-fills its scope picker.
    pub suggested_scopes: Vec<String>,
    /// True when `metadata.registration_endpoint` is set. UI uses this to
    /// hide the "Bring your own OAuth client" fields (DCR will populate
    /// them automatically) or surface them as a fallback.
    pub supports_dcr: bool,
    /// Registered provider whose `authorization_url` host matches the
    /// discovered `issuer`. Includes both built-in catalog providers and
    /// workspace-custom registrations. `None` means the UI should offer
    /// the workspace admin a "Register this provider" inline form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_provider: Option<Provider>,
}

/// How OAuth client credentials are sourced for a provider. Three sources
/// per the docs/specs/oauth-client-sources-review.md model:
///   * `PlatformDefault` — distri's pre-registered creds (env vars).
///     Workspaces may override via BYOK.
///   * `Required` — the workspace must supply its own client_id/secret;
///     distri ships no creds for this provider.
///   * `Dcr` — the provider's auth server publishes a registration
///     endpoint; distri auto-registers per RFC 7591 on first authorize.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ByokPolicy {
    PlatformDefault {
        env_client_id: String,
        env_client_secret: String,
    },
    Required,
    Dcr,
}

/// Canonical declarative description of an OAuth-shaped credential source.
/// Drives the directory tile, the create-form schema-driven inputs, the
/// discovery match, the configure page's per-user extras, and the BYOK
/// policy. Same shape applies to built-in (file catalog) and workspace-
/// custom (DB-backed) providers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct Provider {
    /// Stable identifier. For built-ins this is just the provider name
    /// (`slack`, `github`, …) — the file catalog has no UUID. For
    /// workspace-custom rows this is the DB row's UUID stringified.
    pub id: String,
    /// `None` for built-in catalog entries; `Some(workspace_id)` for
    /// workspace-custom registrations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Uuid>,
    /// Slug used as the OAuth-handler entity key, secret-store namespace
    /// prefix, and connection.auth.provider value. Lowercase
    /// `[a-z0-9_-]+`. Built-ins use the catalog name; workspace-custom
    /// rows are admin-chosen at registration time (defaulted from the
    /// discovered URL host).
    pub name: String,
    /// Friendly label for the UI (`Slack`, `Linear`, …). Defaulted to
    /// `Title Case(name)` when the admin doesn't override.
    pub display_name: String,
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    /// Populated by the discovery flow when the server publishes one,
    /// or by the admin at registration time. Drives `byok_policy = Dcr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Vec<String>,
    /// Extra OAuth-URL params the provider always wants set (e.g.
    /// Google's `access_type=offline`). Merged with caller-supplied
    /// extras at URL build time.
    #[serde(default)]
    pub default_auth_params: HashMap<String, String>,
    /// JSON Schema for caller-overridable extras (e.g. Slack's `team`
    /// workspace ID). UI renders one input per schema property; server
    /// validates extras against this before injecting into the OAuth URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
    pub byok_policy: ByokPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Set on workspace-custom rows whose env vars + DB lookups have
    /// surfaced credentials at runtime. UI uses this to mark the
    /// directory tile as `UNAVAILABLE`. For built-ins this is also the
    /// runtime "env present + secret matches" check.
    #[serde(default)]
    pub available: bool,
}

/// Request body for `POST /v1/connection-providers` — register a
/// workspace-custom OAuth provider that distri doesn't ship in the
/// built-in catalog.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateProviderRequest {
    pub name: String,
    pub display_name: String,
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Vec<String>,
    #[serde(default)]
    pub default_auth_params: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
    pub byok_policy: ByokPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// PATCH body for `PATCH /v1/connection-providers/{id}`. Every field
/// optional — missing fields keep their current value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateProviderRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes_supported: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_auth_params: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byok_policy: Option<ByokPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Response body for `POST /v1/connections/{id}/test` — run a one-shot
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
