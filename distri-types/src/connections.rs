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

/// How a connection resolves identity: workspace-level (default) or per end-user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAuthMode {
    Workspace,
    EndUser,
}

impl std::fmt::Display for ConnectionAuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workspace => write!(f, "workspace"),
            Self::EndUser => write!(f, "end_user"),
        }
    }
}

impl std::str::FromStr for ConnectionAuthMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "workspace" => Ok(Self::Workspace),
            "end_user" => Ok(Self::EndUser),
            _ => Err(anyhow::anyhow!("unknown connection auth_mode: {}", s)),
        }
    }
}

/// Configuration for resolving end-user identity from an external system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type")]
pub enum UserResolutionConfig {
    /// Call an HTTP endpoint to resolve a user token into identity fields.
    #[serde(rename = "user_profile_api")]
    UserProfileApi {
        url: String,
        method: String,
        headers: HashMap<String, String>,
        mapping: UserFieldMapping,
    },
    /// OAuth-based identity resolution (iteration 2 — config defined, not implemented).
    #[serde(rename = "oauth")]
    OAuth {
        client_id: String,
        client_secret_key: String,
        authorize_url: String,
        token_url: String,
        userinfo_url: String,
        scopes: Vec<String>,
        mapping: UserFieldMapping,
    },
    /// Distri-native identity: resolves from channel_identities → members table.
    #[serde(rename = "distri_native")]
    DistriNative,
}

/// JSONPath-based mapping from an API response to end-user identity fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct UserFieldMapping {
    /// JSONPath to extract the external user ID (required).
    pub external_user_id: String,
    /// JSONPath to extract the display name (optional).
    pub display_name: Option<String>,
    /// JSONPath to extract the email (optional).
    pub email: Option<String>,
    /// JSONPath to extract the role (optional).
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
    /// Whether this connection operates at workspace level or per end-user.
    pub auth_mode: ConnectionAuthMode,
    /// How to resolve end-user identity. Only set when auth_mode = EndUser.
    pub user_resolution_config: Option<UserResolutionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct NewConnection {
    pub workspace_id: Uuid,
    pub skill_id: Uuid,
    pub name: String,
    pub status: ConnectionStatus,
    pub config: serde_json::Value,
    pub connected_by: Option<Uuid>,
    pub auth_mode: ConnectionAuthMode,
    pub user_resolution_config: Option<UserResolutionConfig>,
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
