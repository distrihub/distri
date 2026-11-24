//! What a host hears about a run's pauses. The cloud sends the owner a bot message with
//! Retry · Skip · Ask agent · Details; the OSS server logs. Every method has a default that does
//! nothing.

use crate::workflow_store::{PauseInfo, WorkflowExecutionState};
use distri_types::workflow_ops::AttachTarget;

#[async_trait::async_trait]
pub trait WorkflowRunHook: Send + Sync {
    /// The run parked at `pause.step_id`.
    async fn on_paused(&self, _state: &WorkflowExecutionState, _pause: &PauseInfo) {}

    /// The paused step was handed to a person (`{owner: true}` or `{user}`). An attached agent
    /// is started by the engine and does not come here.
    async fn on_attached(
        &self,
        _state: &WorkflowExecutionState,
        _pause: &PauseInfo,
        _target: &AttachTarget,
    ) {
    }

    /// An attached agent finished without resolving the step; `text` is its last answer.
    async fn on_debug_unresolved(
        &self,
        _state: &WorkflowExecutionState,
        _pause: &PauseInfo,
        _agent: &str,
        _text: &str,
    ) {
    }
}
