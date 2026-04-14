use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

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
    pub is_system: bool,
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
