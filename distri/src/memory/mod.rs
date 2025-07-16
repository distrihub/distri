mod types;
use serde::{Deserialize, Serialize};
pub use types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryConfig {
    InMemory,
    File(String),
}
