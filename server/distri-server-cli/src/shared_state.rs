use std::any::Any;
use std::sync::Arc;

pub type SharedState = Arc<dyn Any + Send + Sync>;
