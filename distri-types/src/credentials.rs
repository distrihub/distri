//! Credential entity — the single canonical record of authentication material.
//!
//! Until 2026-05, auth lived directly on `Connection.auth_type`. The bot-setup
//! refactor needs the same credential to be reachable from a `Bot` (as an
//! end-user gate) and a `Connection` (as downstream API auth), so the auth
//! material is now its own row in `credentials`.
//!
//! See `docs/specs/credential-separation.md` for the design rationale,
//! including why this is named `Credential` and not yet another `Auth*`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::connections::{AuthScope, CustomField};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Pending,
    Connected,
    Expired,
    NeedsSetup,
    Error,
    Revoked,
}

impl std::fmt::Display for CredentialStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Connected => write!(f, "connected"),
            Self::Expired => write!(f, "expired"),
            Self::NeedsSetup => write!(f, "needs_setup"),
            Self::Error => write!(f, "error"),
            Self::Revoked => write!(f, "revoked"),
        }
    }
}

impl std::str::FromStr for CredentialStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "connected" => Ok(Self::Connected),
            "expired" => Ok(Self::Expired),
            "needs_setup" => Ok(Self::NeedsSetup),
            "error" => Ok(Self::Error),
            "revoked" => Ok(Self::Revoked),
            _ => Err(anyhow::anyhow!("unknown credential status: {}", s)),
        }
    }
}

/// What kind of authn this Credential holds.
///
/// Replaces the prior `connections::AuthType` enum variant-for-variant so the
/// per-site refactor is a *move + field rename*, not a redesign.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialMaterial {
    /// Standard OAuth flow via the distri OAuth provider registry.
    Oauth {
        provider: String,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// User-defined key/value field schema. Values live in the `secrets`
    /// table under key `credential.<id>.<field_key>`.
    Custom {
        #[serde(default)]
        fields: Vec<CustomField>,
    },
    /// Platform-internal: the seeded distri credential used by the official bot.
    /// No configuration — the caller's distri session token is proxied through.
    DistriNative,
}

impl CredentialMaterial {
    pub fn provider_name(&self) -> &str {
        match self {
            Self::Oauth { provider, .. } => provider.as_str(),
            Self::Custom { .. } => "custom",
            Self::DistriNative => "distri",
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

/// A credentials-table row.
///
/// **BYOK** fields (`oauth_client_id`, `oauth_client_secret`) sit on this
/// outer struct rather than on `CredentialMaterial::Oauth` so the
/// persisted `material` JSON stays a pure metadata enum. The values flow
/// through `secret_store` under `credential.<id>.oauth_client_id` /
/// `_secret`. On READ the service can populate these fields from secrets
/// when a caller needs them (OAuth flow, refresh path); on WRITE the
/// service extracts them and stores them as secrets instead of letting
/// them into the JSON column.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct Credential {
    pub id: Uuid,
    /// `Uuid::nil()` for platform-seeded system rows; otherwise the owning workspace.
    pub workspace_id: Uuid,
    /// UNIQUE per `(workspace_id, name)`.
    pub name: String,
    pub auth_scope: AuthScope,
    pub material: CredentialMaterial,
    pub status: CredentialStatus,
    #[serde(default)]
    pub is_system: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    /// **BYOK** / **DCR**: workspace-provided OAuth `client_id` to use in
    /// place of the platform-managed `provider_registry` entry. `None` =
    /// fall back to the registry. Persisted to `secret_store` keyed
    /// `credential.<id>.oauth_client_id`; hydrated onto this struct by
    /// the service when needed (write paths, secrets-listing endpoints
    /// for the workspace admin). Standard read paths leave it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,

    /// Companion to `oauth_client_id`. Public OAuth clients
    /// (`token_endpoint_auth_method=none`) can omit it. **Never returned
    /// in API responses** — write-only from the caller's perspective.
    /// Persisted to `secret_store` keyed `credential.<id>.oauth_client_secret`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct NewCredential {
    pub workspace_id: Uuid,
    pub name: String,
    pub auth_scope: AuthScope,
    pub material: CredentialMaterial,
    pub status: CredentialStatus,
    #[serde(default)]
    pub is_system: bool,
    pub connected_by: Option<Uuid>,

    /// BYOK OAuth client credentials (see `Credential` doc). The
    /// `CredentialStore::create` impl extracts these and writes them
    /// to `secret_store`; they are NOT persisted in the `material` JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret: Option<String>,
}

/// Typed OAuth/refresh token bundle stored in Redis under
/// `credential:token:{credential_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
pub struct CredentialToken {
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

impl CredentialToken {
    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| exp < Utc::now()).unwrap_or(false)
    }
}
