use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentError {
    pub message: String,
    pub details: Option<serde_json::Value>,
}
