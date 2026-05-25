use std::collections::HashMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::McpClientTransport;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAuthType {
    OAuth2,
    Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Expired,
    NeedsSetup,
    Partial,
    Pending,
    Error,
}

impl std::fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionStatus::Connected => write!(f, "connected"),
            ConnectionStatus::Disconnected => write!(f, "disconnected"),
            ConnectionStatus::Expired => write!(f, "expired"),
            ConnectionStatus::NeedsSetup => write!(f, "needs_setup"),
            ConnectionStatus::Partial => write!(f, "partial"),
            ConnectionStatus::Pending => write!(f, "pending"),
            ConnectionStatus::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for ConnectionStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "connected" => Ok(ConnectionStatus::Connected),
            "disconnected" => Ok(ConnectionStatus::Disconnected),
            "expired" => Ok(ConnectionStatus::Expired),
            "needs_setup" => Ok(ConnectionStatus::NeedsSetup),
            "partial" => Ok(ConnectionStatus::Partial),
            "pending" => Ok(ConnectionStatus::Pending),
            "error" => Ok(ConnectionStatus::Error),
            _ => Err(anyhow::anyhow!("unknown connection status: {}", s)),
        }
    }
}

/// Unified auth scope applied to connections, bots, and tokens.
///
/// - `Public`: anonymous — no auth required. Valid on bots only; tokens never carry Public.
/// - `Workspace`: a logged-in platform member (distri signup via OTP).
/// - `User`: an external actor resolved via a connection (customer's end-user).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    Public,
    Workspace,
    User,
}

impl std::fmt::Display for AuthScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::Workspace => write!(f, "workspace"),
            Self::User => write!(f, "user"),
        }
    }
}

impl std::str::FromStr for AuthScope {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "workspace" => Ok(Self::Workspace),
            "user" => Ok(Self::User),
            _ => Err(anyhow::anyhow!("unknown auth_scope: {}", s)),
        }
    }
}

/// One configurable field on a Custom-auth connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
pub struct CustomField {
    /// Stable identifier used as the secret key suffix (e.g. `api_key`).
    pub key: String,
    /// Optional human-readable label for the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Mask the input in the UI and redact in logs.
    #[serde(default)]
    pub is_secret: bool,
    /// Whether a value is required for the connection to be considered configured.
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

/// Category for grouping Directory tiles. Entries that share a group
/// cluster under one heading (e.g. `Google` covers the vanilla `google`
/// tile + the four `*_mcp` Workspace tiles). Enumerated so the UI's
/// grouping logic and section headings stay type-safe — adding a tag
/// requires editing this enum, not just typing a new string in JSON.
///
/// `Other` is the catch-all for entries we haven't categorised
/// explicitly; the UI renders them in a trailing "Other" section.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq, Eq, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderGroup {
    Google,
    Slack,
    Github,
    Notion,
    Microsoft,
    Twitter,
    Linear,
    Atlassian,
    Discord,
    #[serde(other)]
    Other,
}

impl ProviderGroup {
    /// Heading rendered above the group's tiles in the Directory.
    pub fn display(&self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Slack => "Slack",
            Self::Github => "GitHub",
            Self::Notion => "Notion",
            Self::Microsoft => "Microsoft",
            Self::Twitter => "Twitter / X",
            Self::Linear => "Linear",
            Self::Atlassian => "Atlassian",
            Self::Discord => "Discord",
            Self::Other => "Other",
        }
    }
}

/// Full OAuth provider declaration carried inline on a Connection.
///
/// One canonical shape across three sources:
///   * **Built-in catalog** (`additional_providers.json`) — seeded into
///     the create form when the user picks a known provider tile.
///   * **MCP discovery** (RFC 8414 + 9728) — seeded from
///     `POST /v1/connections/oauth/discover` when the user pastes an
///     MCP URL whose server publishes auth-server metadata.
///   * **Admin-entered** — workspace admin types the values into the
///     custom-connection form directly.
///
/// Once a Connection is created, the config is *frozen* on the row —
/// catalog edits do not retroactively apply. Use
/// `POST /v1/connections/{id}/resync-provider` to re-apply the catalog
/// over an existing connection.
///
/// **Always USER-scoped.** Connections authorize an individual end-user's
/// identity at the third party (Slack `xoxp-…`, Google as user X, …).
/// Bot install flows use channel-setup paths (`channels.bot_token`),
/// NOT this config.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
pub struct OAuthProviderConfig {
    /// Stable slug (`slack`, `github`, `linear`, …) — drives the
    /// catalog-resync lookup and the secret-store namespace prefix.
    pub name: String,
    /// Friendly label for the UI. Falls back to title-cased `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    /// RFC 7591 registration endpoint, when the auth server publishes one
    /// (discovery sets this; catalog entries usually don't).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub default_scopes: Vec<String>,
    /// Extra auth-URL query params the provider always wants set
    /// (Google's `access_type=offline`, Twitter PKCE flag, …).
    #[serde(default)]
    pub default_auth_params: HashMap<String, String>,
    /// JSON Schema describing caller-overridable auth-URL params
    /// (Slack's `team`, Microsoft's `tenant`, …). UI auto-renders inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_params_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub pkce_required: bool,
    /// Env-var names that hold the platform's pre-registered OAuth client
    /// (`SLACK_CLIENT_ID`/`_SECRET`). Both `None` ⇒ no platform creds,
    /// BYOK required. UI offers BYOK alongside platform when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Directory tile wire shape served by `GET /v1/connections/providers`.
/// Tagged union on `kind` so the UI's MCP-vs-REST switching is
/// exhaustive (no nullable `transport_url` to leak through type holes).
///
/// **Rest** — vanilla OAuth (Google OIDC, GitHub, Notion). Workflows
/// use the stored token against the provider's REST API directly.
///
/// **Mcp** — OAuth + pinned `transport_url`. Picking this tile creates
/// an MCP-kind Connection with `Connection.kind.mcp.transport.url`
/// pre-pinned. The OAuth fields are still used for the consent flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogProvider {
    Rest {
        #[serde(flatten)]
        oauth: OAuthProviderConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<ProviderGroup>,
    },
    Mcp {
        #[serde(flatten)]
        oauth: OAuthProviderConfig,
        /// Streamable-HTTP endpoint the connection's MCP transport will
        /// be pinned to.
        transport_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<ProviderGroup>,
    },
}

impl CatalogProvider {
    /// Borrow the OAuth bag shared by both variants.
    pub fn oauth(&self) -> &OAuthProviderConfig {
        match self {
            Self::Rest { oauth, .. } | Self::Mcp { oauth, .. } => oauth,
        }
    }

    /// Borrow the group label shared by both variants.
    pub fn group(&self) -> Option<ProviderGroup> {
        match self {
            Self::Rest { group, .. } | Self::Mcp { group, .. } => *group,
        }
    }

    /// Borrow the pinned MCP transport URL when the entry is MCP-flavored.
    pub fn transport_url(&self) -> Option<&str> {
        match self {
            Self::Mcp { transport_url, .. } => Some(transport_url.as_str()),
            _ => None,
        }
    }
}

impl OAuthProviderConfig {
    /// Effective display name — friendly label or title-cased slug.
    pub fn display(&self) -> String {
        self.display_name.clone().unwrap_or_else(|| {
            let mut chars = self.name.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
    }

    /// Project this provider config to the `AuthType::OAuth2 { ... }`
    /// shape consumed by `OAuthHandler`. `scopes` is the per-connection
    /// requested scopes (different from the catalog's `scopes_supported`).
    pub fn to_auth_type(&self, scopes: Vec<String>) -> crate::auth::AuthType {
        crate::auth::AuthType::OAuth2 {
            flow_type: crate::auth::OAuth2FlowType::AuthorizationCode,
            authorization_url: self.authorization_url.clone(),
            token_url: self.token_url.clone(),
            refresh_url: self.refresh_url.clone(),
            scopes,
            send_redirect_uri: true,
        }
    }
}

/// What kind of authn this Connection holds. Lives directly on the Connection
/// row — auth is connection-shaped, not a shared entity.
///
/// Replaces the prior split `Credential.material` enum.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionAuth {
    /// No auth needed. Used by open MCP servers and test connections.
    None,
    /// OAuth (platform-managed, BYOK, or DCR for MCP). The provider's full
    /// declaration (auth URL, token URL, scopes, PKCE, env-var refs for
    /// platform client creds, …) is carried inline so the connection is
    /// self-sufficient: no runtime catalog lookup needed for OAuth flow.
    Oauth {
        provider: OAuthProviderConfig,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// User-supplied named fields (API keys, custom headers). Values live in
    /// the `secrets` table under key `connection.<id>.<field_key>`.
    Custom {
        #[serde(default)]
        fields: Vec<CustomField>,
    },
    /// Distri's own session token IS the auth.
    DistriNative,
}

impl ConnectionAuth {
    pub fn provider_name(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Oauth { provider, .. } => provider.name.as_str(),
            Self::Custom { .. } => "custom",
            Self::DistriNative => "distri",
        }
    }

    /// Borrow the inline OAuth provider config when this is the `Oauth` variant.
    pub fn oauth_config(&self) -> Option<&OAuthProviderConfig> {
        match self {
            Self::Oauth { provider, .. } => Some(provider),
            _ => None,
        }
    }

    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::Oauth { .. })
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom { .. })
    }

    pub fn is_distri_native(&self) -> bool {
        matches!(self, Self::DistriNative)
    }

    pub fn custom_fields(&self) -> &[CustomField] {
        match self {
            Self::Custom { fields } => fields,
            _ => &[],
        }
    }

    pub fn custom_required_fields(&self) -> Vec<&CustomField> {
        match self {
            Self::Custom { fields } => fields.iter().filter(|f| f.required).collect(),
            _ => vec![],
        }
    }
}

/// Typed OAuth/refresh token bundle stored in Redis under
/// `connection:token:{connection_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct ConnectionToken {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl ConnectionToken {
    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| exp < Utc::now()).unwrap_or(false)
    }
}

/// What capability surface a connection exposes.
///
/// `Default` is the historical behavior: auth-only — the connection contributes
/// credentials to the agent's env vars or to the outbound HTTP proxy.
///
/// `Mcp` makes the connection *also* a remote tool source. Agents reference it
/// by `Connection.name` in `ToolsConfig.mcp[].server`; the executor builds an
/// `McpClientPool` from every `kind = Mcp` connection in scope, with auth
/// (`auth_type`) injected as the transport's bearer header at connect time.
///
/// Auth and capability are orthogonal: the same OAuth provider can back both a
/// `Default` GitHub connection (REST proxy) and a `Mcp` GitHub connection
/// (Streamable HTTP) — they're two rows that share `auth_type.provider`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionKind {
    /// REST / CLI proxy connection. Agents call the API documented in
    /// `skill_content`; distri injects the credential's headers via
    /// `/proxy/request` or the `inject_connection_env` tool.
    Default {
        /// Markdown skill describing the API surface. Required for custom
        /// connections; optional for built-in OAuth providers that ship a
        /// bundled template (the server fills in from
        /// `connection_skill_templates/<provider>.md` when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_content: Option<String>,
    },
    /// Remote MCP server. distri connects to `transport.url` and exposes
    /// its tools to agents that reference this connection's name in
    /// `tools.mcp[].server`.
    Mcp {
        #[serde(flatten)]
        mcp: McpConnectionSpec,
    },
}

impl Default for ConnectionKind {
    fn default() -> Self {
        Self::Default {
            skill_content: None,
        }
    }
}

impl ConnectionKind {
    pub fn is_mcp(&self) -> bool {
        matches!(self, Self::Mcp { .. })
    }
    pub fn as_mcp(&self) -> Option<&McpConnectionSpec> {
        match self {
            Self::Mcp { mcp } => Some(mcp),
            _ => None,
        }
    }
    /// Skill markdown if this is a Default-kind connection that ships one.
    /// MCP-kind connections always return `None` (their tool surface is
    /// the MCP server's `tools/list`, not a skill doc).
    pub fn skill_content(&self) -> Option<&str> {
        match self {
            Self::Default { skill_content } => skill_content.as_deref(),
            Self::Mcp { .. } => None,
        }
    }
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Default { .. } => "default",
            Self::Mcp { .. } => "mcp",
        }
    }
}

/// MCP-specific configuration carried on `ConnectionKind::Mcp`.
///
/// The transport is restricted to remote variants in the UI (Streamable HTTP /
/// SSE) — stdio is intentionally not user-configurable because connections are
/// meant to be portable across hosts. `extra_headers` are merged with the
/// resolver-injected `Authorization` header at connect time.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct McpConnectionSpec {
    pub transport: McpClientTransport,
    /// Optional human description shown in the UI list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Include/exclude glob patterns applied to discovered tool names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_filter: Option<McpToolFilter>,
    /// Whether the server is enabled for tool resolution.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct McpToolFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct Connection {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub skill_id: Uuid,
    pub name: String,
    pub status: ConnectionStatus,
    pub config: serde_json::Value,
    pub connected_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Who is allowed to use this connection. Workspace = platform members,
    /// EndUser = external actors resolved via a handshake.
    pub auth_scope: AuthScope,
    /// The authentication material this connection carries. Auth is
    /// connection-shaped — there is no separate Credential entity.
    pub auth: ConnectionAuth,
    /// What surface this connection exposes (auth-only vs MCP tool source).
    #[serde(default)]
    pub kind: ConnectionKind,
    /// Platform-seeded connections (e.g. the `distri` connection) carry is_system=true
    /// and are write-protected from user mutations.
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct NewConnection {
    pub workspace_id: Uuid,
    pub skill_id: Uuid,
    pub name: String,
    pub status: ConnectionStatus,
    pub config: serde_json::Value,
    pub connected_by: Option<Uuid>,
    pub auth_scope: AuthScope,
    pub auth: ConnectionAuth,
    #[serde(default)]
    pub kind: ConnectionKind,
    #[serde(default)]
    pub is_system: bool,
}

impl Connection {
    pub fn is_mcp(&self) -> bool {
        self.kind.is_mcp()
    }
    pub fn mcp_spec(&self) -> Option<&McpConnectionSpec> {
        self.kind.as_mcp()
    }
}

/// Admin-authored HTTP probe attached to a *connection* — used by the
/// "New Custom Connection" UI to confirm the configured URL + supplied
/// auth fields actually reach the downstream service before save.
///
/// Stored in `connection.config['verify_request']`. Scope is intentionally
/// per-connection: a connection is "this URL + this transport + this auth",
/// and the probe tests that combination. Unrelated to bot gating (which is
/// just "does this end-user hold valid auth for this connection";
/// see `Bot.gate_connection_id`).
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyRequest {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl Connection {
    /// Returns the `verify_request` object from `config`, if present.
    pub fn verify_request(&self) -> Option<VerifyRequest> {
        self.config
            .get("verify_request")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

/// Declarative reference to a connection that an agent definition requires.
///
/// Resolved at agent-run start: the orchestrator matches this against the
/// workspace's `connections` table, fetches the secret (OAuth token / custom
/// fields / distri-native session), and injects the result into
/// `ExecutorContext.env_vars` + `dynamic_values.available_connections`.
///
/// Prefer `provider` (portable across workspaces) over `connection_id`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema, Default)]
pub struct ConnectionRequirement {
    /// Match by provider name (preferred): "google", "slack", ...
    /// Resolved against the Connection's `auth.provider` for `Oauth`,
    /// the Connection's name for `Custom`, and `"distri"` for `DistriNative`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Pin to a specific connection ID. Takes precedence over `provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<Uuid>,

    /// Minimum OAuth scopes required. Resolution fails (when `required=true`)
    /// or marks the requirement unmet (when `required=false`) if the connected
    /// token doesn't cover all of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,

    /// Env var name override. Default: `<PROVIDER>_TOKEN` for OAuth,
    /// `<PROVIDER>_<FIELD_KEY>` for each Custom field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,

    /// If true, the agent fails to start when this connection can't be resolved.
    /// If false (default), the agent starts and the requirement is surfaced in
    /// `{{available_providers}}` so the LLM can prompt the user to connect.
    #[serde(default)]
    pub required: bool,
}
