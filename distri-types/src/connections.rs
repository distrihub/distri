use std::collections::HashMap;

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

/// Unified auth scope applied to tokens, connections, and channels.
///
/// - `Public`: anonymous — no auth required. Valid on channels only; tokens never carry Public.
/// - `Workspace`: a logged-in platform member (distri signup via OTP).
/// - `EndUser`: an external actor resolved via a connection (customer's end-user).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    Public,
    Workspace,
    EndUser,
}

impl std::fmt::Display for AuthScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::Workspace => write!(f, "workspace"),
            Self::EndUser => write!(f, "end_user"),
        }
    }
}

impl std::str::FromStr for AuthScope {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "workspace" => Ok(Self::Workspace),
            "end_user" => Ok(Self::EndUser),
            _ => Err(anyhow::anyhow!("unknown auth_scope: {}", s)),
        }
    }
}

/// How a connection authenticates to the downstream API.
///
/// OAuth/BearerToken/ApiKey are user-creatable. DistriNative is system-seeded only —
/// it represents the platform-internal distri connection used by the official bot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthType {
    /// Standard OAuth flow via the distri OAuth provider registry.
    OAuth {
        provider: String,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Bearer token header. Default header `Authorization: Bearer <token>`, customizable.
    BearerToken {
        #[serde(default = "default_authorization_header")]
        header_name: String,
        #[serde(default = "default_bearer_prefix")]
        prefix: String,
    },
    /// Custom API key header. e.g. `X-API-Key: <key>`.
    ApiKey { header_name: String },
    /// Platform-internal: the seeded distri connection used by the official bot.
    /// No configuration — the caller's distri session token is proxied through.
    DistriNative,
}

fn default_authorization_header() -> String {
    "Authorization".to_string()
}

fn default_bearer_prefix() -> String {
    "Bearer ".to_string()
}

impl AuthType {
    /// Provider name used for skill template lookup and connection display.
    pub fn provider_name(&self) -> &str {
        match self {
            Self::OAuth { provider, .. } => provider.as_str(),
            Self::BearerToken { .. } => "bearer",
            Self::ApiKey { .. } => "api_key",
            Self::DistriNative => "distri",
        }
    }

    pub fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuth { .. })
    }

    pub fn is_distri_native(&self) -> bool {
        matches!(self, Self::DistriNative)
    }
}

/// Optional HTTP call that enriches the end-user identity at handshake time.
/// Only meaningful for connections with `auth_scope = EndUser`. When configured,
/// the channel-auth flow calls this endpoint with the user's submitted secret
/// interpolated into the request. A 2xx response marks the binding confirmed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct UserProfileRetrieval {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndUserAuthStatus {
    Pending,
    Confirmed,
    Failed,
}

impl std::fmt::Display for EndUserAuthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Confirmed => write!(f, "confirmed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// JSONPath-based mapping from an API response to end-user identity fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct UserFieldMapping {
    /// JSONPath to extract the external user ID (required).
    pub external_user_id: String,
    /// JSONPath to extract the display name (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// JSONPath to extract the email (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// JSONPath to extract the role (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
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
    /// Optional enrichment config invoked on EndUser handshake; only meaningful for
    /// EndUser-scope connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_profile_retrieval: Option<UserProfileRetrieval>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_profile_retrieval: Option<UserProfileRetrieval>,
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
