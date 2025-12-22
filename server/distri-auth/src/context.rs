/// Trait describing contextual data required by the distri authentication stack.
/// Host applications can implement this trait to expose user-context metadata
/// without creating direct dependencies on their concrete types.
pub trait ToolAuthRequestContext: Send + Sync {
    fn user_id(&self) -> String;
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
