mod auth;
pub mod external_tool_calls;
pub mod prompt;
pub mod workflow;
use std::collections::HashMap;

pub use auth::*;
// Re-export the main store traits and types
pub use external_tool_calls::*;

pub mod diesel_store;
pub mod models;
pub mod schema;

// Re-export prompt template store types
pub use diesel_store::{DieselPromptTemplateStore, DieselSecretStore};

/// Initialize all stores based on configuration
mod initialize;
pub use initialize::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalSession {
    pub values: HashMap<String, serde_json::Value>,
    pub expiries: HashMap<String, chrono::DateTime<chrono::Utc>>,
}
