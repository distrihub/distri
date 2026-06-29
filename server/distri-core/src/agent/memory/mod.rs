use std::{collections::HashMap, sync::Arc};

use distri_types::{ExecutionResult, PlanStep};
use serde::{Deserialize, Serialize};

use crate::{
    agent::{strategy::execution::MemoryStrategy, ExecutorContext},
    AgentError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryConfig {
    InMemory,
    File(String),
}

#[derive(Debug)]
pub struct DefaultMemoryManager {
    pub memory: HashMap<String, String>,
}

impl DefaultMemoryManager {
    pub fn new() -> Self {
        Self {
            memory: HashMap::new(),
        }
    }
}

#[async_trait::async_trait]

impl MemoryStrategy for DefaultMemoryManager {
    async fn load_memory(&self, _context: Arc<ExecutorContext>) -> Result<(), AgentError> {
        Ok(())
    }

    async fn store_step_result(
        &self,
        _step: &PlanStep,
        _result: &ExecutionResult,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    async fn summarize(&self) -> Result<Option<String>, AgentError> {
        Ok(None)
    }
}
