use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Registry package download response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackageResponse {
    pub package: String,
    pub version: String,
    pub artifact: serde_json::Value,
    pub tarball: String, // base64-encoded tarball
    pub checksum: String,
    pub download_url: String,
    pub metadata: RegistryPackageMetadata,
}

/// Registry package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackageMetadata {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_yanked: bool,
}

/// Registry package version information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackageVersion {
    pub id: Uuid,
    pub package_id: Uuid,
    pub version: String,
    pub artifact: serde_json::Value,
    pub tarball: String, // base64-encoded
    pub checksum: String,
    pub is_yanked: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Registry package information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackage {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub owner_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Registry package detail with versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackageDetail {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub owner_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub versions: Vec<RegistryPackageVersion>,
    pub latest_version: Option<String>,
}

/// Registry search query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySearchQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
}

/// Registry search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySearchResults {
    pub packages: Vec<RegistryPackage>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
    pub total_pages: i64,
}

/// Registry publish request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPublishRequest {
    pub name: String,
    pub version: String,
    pub artifact: serde_json::Value,
    pub tarball: String, // base64-encoded
}

/// Registry publish response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPublishResponse {
    pub message: String,
    pub package_id: Uuid,
    pub version_id: Uuid,
}

/// Registry yank request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryYankRequest {
    pub yanked: bool,
}

/// Registry yank response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryYankResponse {
    pub message: String,
    pub version: String,
    pub yanked: bool,
}

/// Registry API error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryApiError {
    pub error: String,
    pub message: String,
}

/// Registry user information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryUserInfo {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub email_verified: bool,
}

/// Registry login request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryLoginRequest {
    pub email: String,
    pub password: String,
}

/// Registry login response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryLoginResponse {
    pub message: String,
    pub token: String,
    pub user: RegistryUserInfo,
}

/// Registry register request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryRegisterRequest {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
}
