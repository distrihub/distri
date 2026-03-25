//! distri-workflow — workflow engine for Distri.
//!
//! Define multi-step workflows as data, execute them step by step,
//! with support for sequential/parallel execution, conditions, agent runs,
//! and persistent state tracking.
//!
//! # Architecture
//!
//! - `WorkflowDefinition` — the workflow as a DAG of steps
//! - `WorkflowStep` — a single step (API call, script, agent run, condition)
//! - `StepExecutor` trait — executes a step (implement for your runtime)
//! - `WorkflowStateStore` trait — persists workflow state (Redis, DB, in-memory)
//! - `WorkflowRunner` — orchestrates execution
//!
//! # Example
//!
//! ```rust,no_run
//! use distri_workflow::*;
//!
//! let steps = vec![
//!     WorkflowStep::api_call("read", "Read document", "GET", "/api/docs/{id}"),
//!     WorkflowStep::agent_run("detect", "Detect content", "detector_agent", "Analyze this document"),
//!     WorkflowStep::api_call("save", "Save results", "POST", "/api/results")
//!         .with_depends_on(vec!["read", "detect"]),
//! ];
//!
//! let workflow = WorkflowDefinition::new("import", steps)
//!     .with_context(serde_json::json!({ "doc_id": "123" }));
//! ```

pub mod types;
pub mod store;
pub mod executor;

pub use types::*;
pub use store::{WorkflowStateStore, InMemoryStore};
pub use executor::{StepExecutor, WorkflowRunner};

#[cfg(test)]
mod tests;
