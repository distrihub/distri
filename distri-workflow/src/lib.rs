//! distri-workflow — workflow engine for Distri.
//!
//! Define multi-step workflows as data, execute them step by step,
//! with support for sequential/parallel execution, conditions, agent runs,
//! tool calls, and persistent state tracking.
//!
//! # Architecture
//!
//! - `WorkflowDefinition` — the workflow as a DAG of steps
//! - `WorkflowStep` — a single step (API call, script, agent run, tool call, condition)
//! - `StepRequirement` — what a step needs to run (native skills, connections)
//! - `StepExecutor` trait — executes a step (implement for your runtime)
//! - `WorkflowStateStore` trait — persists workflow state (Redis, DB, in-memory)
//! - `WorkflowRunner` — orchestrates execution with requirement checking
//!
//! # Example
//!
//! ```rust,no_run
//! use distri_workflow::*;
//!
//! let steps = vec![
//!     WorkflowStep::api_call("read", "Read document", "GET", "/api/docs/{id}")
//!         .with_requires(vec![
//!             StepRequirement::native("network"),
//!             StepRequirement::connection("google", "drive").with_permissions(vec!["drive.readonly"]),
//!         ]),
//!     WorkflowStep::tool_call("process", "Process data", "analyze_doc", serde_json::json!({"format": "markdown"})),
//!     WorkflowStep::script("test", "Run tests", "cargo test")
//!         .with_cwd("/project")
//!         .with_timeout(300),
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
