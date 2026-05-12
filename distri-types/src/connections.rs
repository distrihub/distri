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

/// How a connection authenticates to the downstream API.
///
/// OAuth and Custom are user-creatable. DistriNative is system-seeded only —
/// it represents the platform-internal distri connection used by the official bot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthType {
    /// Standard OAuth flow via the distri OAuth provider registry.
    /// Explicit rename so serde doesn't snake_case `OAuth` → `o_auth`.
    #[serde(rename = "oauth")]
    OAuth {
        provider: String,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// User-defined key/value field schema. The admin declares the shape; values
    /// are collected separately (inline at create time for Workspace scope, or
    /// via the configure URL for User scope) and stored as rows in the
    /// `secrets` table under key `connection.<id>.<field_key>`.
    Custom {
        #[serde(default)]
        fields: Vec<CustomField>,
    },
    /// Platform-internal: the seeded distri connection used by the official bot.
    /// No configuration — the caller's distri session token is proxied through.
    DistriNative,
}

/// One configurable field on a Custom connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
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

impl AuthType {
    /// Provider name used for skill template lookup and connection display.
    pub fn provider_name(&self) -> &str {
        match self {
            Self::OAuth { provider, .. } => provider.as_str(),
            Self::Custom { .. } => "custom",
            Self::DistriNative => "distri",
        }
    }

    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuth { .. })
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom { .. })
    }

    pub fn is_distri_native(&self) -> bool {
        matches!(self, Self::DistriNative)
    }

    /// Shorthand for iterating required Custom fields (empty for OAuth/DistriNative).
    pub fn custom_required_fields(&self) -> Vec<&CustomField> {
        match self {
            Self::Custom { fields } => fields.iter().filter(|f| f.required).collect(),
            _ => vec![],
        }
    }

    /// All Custom fields regardless of required-ness.
    pub fn custom_fields(&self) -> &[CustomField] {
        match self {
            Self::Custom { fields } => fields,
            _ => &[],
        }
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
    Default,
    Mcp {
        #[serde(flatten)]
        mcp: McpConnectionSpec,
    },
}

impl Default for ConnectionKind {
    fn default() -> Self {
        Self::Default
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
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
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
    /// How the connection authenticates to the downstream API.
    pub auth_type: AuthType,
    /// What surface this connection exposes (auth-only vs MCP tool source).
    /// Defaults to `Default` for backward-compat with existing rows that
    /// didn't carry the field.
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
    pub auth_type: AuthType,
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

/// Typed OAuth token — replaces raw serde_json::Value.
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

/// Describes an HTTP request that the gateway should make to verify a user's
/// identity against an external service. Stored in `connection.config` under
/// the key `verify_request`.
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
    /// Resolved against `AuthType::OAuth.provider` / `AuthType::Custom` (name match)
    /// / `AuthType::DistriNative` (provider_name == "distri").
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
