mod types;
use serde::{Deserialize, Serialize};
pub use types::*;
pub mod build;
pub mod file_memory_store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryConfig {
    InMemory,
    File(String),
}
