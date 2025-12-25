use std::future::Future;

/// Trait describing contextual data required by the distri authentication stack.
/// Host applications can implement this trait to expose user-context metadata
/// without creating direct dependencies on their concrete types.
pub trait ToolAuthRequestContext: Send + Sync {
    fn user_id(&self) -> String;
}

tokio::task_local! {
    static CURRENT_USER_ID: String;
}

/// Basic context implementation carrying only a user id.
#[derive(Clone)]
pub struct UserContext {
    user_id: String,
}

impl UserContext {
    pub fn new(user_id: String) -> Self {
        Self { user_id }
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

pub async fn with_user_id<F, T>(user_id: String, fut: F) -> T
where
    F: Future<Output = T>,
{
    CURRENT_USER_ID.scope(user_id, fut).await
}
