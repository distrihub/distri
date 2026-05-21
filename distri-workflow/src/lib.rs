//! distri-workflow — workflow types + sidecar stores for Distri.
//!
//! After the spec rewrite that unified workflow orchestration with the
//! task system, this crate is the *data layer*:
//!
//! - **Definition** — the workflow as a DAG of steps. Static template;
//!   no runtime state. (`WorkflowDefinition`, `WorkflowStep`).
//! - **Run-level sidecar** — what a bare `Task` row can't carry:
//!   definition snapshot, entry point, input, shared context.
//!   (`WorkflowExecutionState`).
//! - **Step-level sidecar** — per-step status, result, error,
//!   timestamps, and the optional `wait_task_id` for wait-style steps
//!   that need to be A2A-addressable. (`WorkflowStepState`).
//! - **Store trait** — single CRUD surface over both sidecars
//!   (`WorkflowStore`). In-memory and Redis impls.
//! - **Trigger registry** — routing index from declared workflow
//!   triggers (webhook path / cron / event topic / tool name) back to
//!   `(agent_id, entry_point_id)`. (`WorkflowTriggerRegistry`).
//! - **In-memory runtime aggregate** — `WorkflowRun` + `WorkflowStepRun`
//!   are the transient in-process view used by the workflow agent's
//!   step driver. The driver lives in `distri-core` (see
//!   `agent/workflow_driver.rs`) — there is no `WorkflowRunner` /
//!   `WorkflowStateStore` / `InMemoryStore` / `EventSink` /
//!   `WorkflowEvent` parallel substrate anymore; everything flows
//!   through the canonical `TaskStore` + `AgentEventBroadcaster` +
//!   `WorkflowStore` triple.

pub mod resolve;
pub mod trigger_registry;
pub mod types;
pub mod workflow_store;

pub use resolve::{
    build_execution_context, evaluate_skip_condition, resolve_step_input, resolve_template,
    resolve_value,
};
pub use trigger_registry::{
    InMemoryWorkflowTriggerRegistry, TriggerBinding, WorkflowTriggerRegistry,
};
pub use types::*;
pub use workflow_store::{
    InMemoryWorkflowStore, WorkflowExecutionState, WorkflowStepState, WorkflowStore,
};

#[cfg(test)]
mod tests;
