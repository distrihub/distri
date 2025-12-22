use std::sync::Arc;

use crate::{agent::ExecutorContext, AgentError};
#[cfg(test)]
mod tests;

use distri_types::{ExecutionResult, PlanStep};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Memory record for storing step results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct MemoryRecord {
    pub step_id: String,
    pub timestamp: i64,
    pub message: String,
    pub notes: Option<String>, // summary or reflections
}

/// Reflection log for post-execution analysis
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ReflectionLog {
    pub summary: String,
    pub related_steps: Vec<String>,
    pub suggestions: Vec<String>,
    pub timestamp: i64,
}

/// Base execution strategy trait
#[async_trait::async_trait]
pub trait ExecutionStrategy: Send + Sync + std::fmt::Debug {
    async fn execute_step(
        &self,
        step: &PlanStep,
        context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError>;

    async fn execute_step_stream(
        &self,
        step: &PlanStep,
        context: Arc<ExecutorContext>,
    ) -> Result<ExecutionResult, AgentError>;

    async fn should_continue(
        &self,
        plan: &[PlanStep],
        current_index: usize,
        context: Arc<ExecutorContext>,
    ) -> bool;
}

pub mod default;

pub use default::AgentExecutor;

/// Memory management strategy trait
#[async_trait::async_trait]
pub trait MemoryStrategy: Send + Sync + std::fmt::Debug {
    async fn load_memory(&self, context: Arc<ExecutorContext>) -> Result<(), AgentError>;

    async fn store_step_result(
        &self,
        step: &PlanStep,
        result: &ExecutionResult,
    ) -> Result<(), AgentError>;

    async fn summarize(&self) -> Result<Option<String>, AgentError>;
}

/// Reflection strategy trait
#[async_trait::async_trait]
pub trait ReflectionStrategy: Send + Sync + std::fmt::Debug {
    async fn reflect(
        &self,
        task: &str,
        history: &[ExecutionResult],
        context: Arc<ExecutorContext>,
    ) -> Result<Option<String>, AgentError>;
}
