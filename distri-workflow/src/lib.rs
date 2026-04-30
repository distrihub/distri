//! distri-workflow — workflow engine for Distri.
//!
//! Define multi-step workflows as data, execute them step by step,
//! with support for sequential/parallel execution, conditions, agent runs,
//! tool calls, and persistent state tracking.
//!
//! # Architecture
//!
//! Two layers, deliberately separated:
//!
//! - **Definition** — the workflow as a DAG of steps. Static template;
//!   no runtime state. (`WorkflowDefinition`, `WorkflowStep`).
//! - **Run** — execution state for one invocation: status, shared
//!   context, per-step status / result / error / timestamps.
//!   (`WorkflowRun`, `WorkflowStepRun`).
//!
//! Other key types:
//!
//! - `StepRequirement` — what a step needs to run (native skills, connections)
//! - `StepExecutor` trait — executes a step (implement for your runtime)
//! - `WorkflowStateStore` trait — persists `WorkflowRun`s (Redis, DB, in-memory)
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
//! let definition = WorkflowDefinition::new(steps);
//! let run = WorkflowRun::new(definition);
//! ```

pub mod executor;
pub mod resolve;
pub mod step_executions;
pub mod store;
pub mod types;

pub use executor::{EventSink, NoopEventSink, StepExecutor, TracingEventSink, WorkflowRunner};
pub use resolve::{
    build_execution_context, evaluate_skip_condition, resolve_step_input, resolve_template,
    resolve_value,
};
pub use step_executions::{
    InMemoryWorkflowStepExecutionStore, WorkflowStepExecution, WorkflowStepExecutionStore,
    WorkflowStepExecutionUpdate,
};
pub use store::{InMemoryStore, WorkflowStateStore};
pub use types::*;

#[cfg(test)]
mod tests;
