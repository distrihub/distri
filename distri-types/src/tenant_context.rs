use serde::{Deserialize, Serialize};

/// Context for multi-tenant operations
/// Holds user and workspace information needed for tenant filtering in stores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantContext {
    pub user_id: String,
    pub workspace_id: Option<uuid::Uuid>,
}

impl TenantContext {
    pub fn new(user_id: String, workspace_id: Option<uuid::Uuid>) -> Self {
        Self {
            user_id,
            workspace_id,
        }
    }

    pub fn anonymous() -> Self {
        Self {
            user_id: "anonymous".to_string(),
            workspace_id: None,
        }
    }
}

impl Default for TenantContext {
    fn default() -> Self {
        Self::anonymous()
    }
}
