use std::future::Future;
use uuid::Uuid;

/// Trait describing contextual data required by the distri authentication stack.
/// Host applications can implement this trait to expose user-context metadata
/// without creating direct dependencies on their concrete types.
pub trait ToolAuthRequestContext: Send + Sync {
    fn user_id(&self) -> String;
}

tokio::task_local! {
    static CURRENT_USER_ID: String;
    static CURRENT_WORKSPACE_ID: Uuid;
}

/// Basic context implementation carrying user id and optional workspace id.
#[derive(Clone)]
pub struct UserContext {
    user_id: String,
    workspace_id: Option<String>,
}

impl UserContext {
    pub fn new(user_id: String) -> Self {
        Self { user_id, workspace_id: None }
    }

    pub fn with_workspace(user_id: String, workspace_id: Option<String>) -> Self {
        Self { user_id, workspace_id }
    }

    pub fn workspace_id(&self) -> Option<String> {
        self.workspace_id.clone()
    }
}

impl ToolAuthRequestContext for UserContext {
    fn user_id(&self) -> String {
        self.user_id.clone()
    }
}

pub fn current_user_id() -> Option<String> {
    CURRENT_USER_ID.try_with(|id| id.clone()).ok()
}

/// Get the current workspace ID from task-local context (distri-auth level).
pub fn current_workspace_id() -> Option<Uuid> {
    CURRENT_WORKSPACE_ID.try_with(|id| *id).ok()
}

pub async fn with_user_id<F, T>(user_id: String, fut: F) -> T
where
    F: Future<Output = T>,
{
    CURRENT_USER_ID.scope(user_id, fut).await
}

/// Run a future with both user_id and workspace_id set in task-local context.
pub async fn with_user_and_workspace<F, T>(user_id: String, workspace_id: Option<Uuid>, fut: F) -> T
where
    F: Future<Output = T>,
{
    match workspace_id {
        Some(ws_id) => {
            CURRENT_USER_ID
                .scope(user_id, CURRENT_WORKSPACE_ID.scope(ws_id, fut))
                .await
        }
        None => CURRENT_USER_ID.scope(user_id, fut).await,
    }
}
