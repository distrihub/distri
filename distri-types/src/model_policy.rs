//! A hook that decides which models a run may call.
//!
//! The orchestrator asks it once the agent's final model is known — after the
//! workspace default, the agent's own `model_settings` pin and any runtime
//! override are merged — so no route to the LLM can pick a model the
//! workspace has not allowed. The multi-tenant cloud wires one that reads the
//! workspace's `allowed_models`; standalone `distri-server` has none.

use async_trait::async_trait;

/// Error prefix the cloud and channel gateway recognise.
pub const MODEL_NOT_ALLOWED: &str = "model_not_allowed";

#[async_trait]
pub trait ModelPolicy: Send + Sync + 'static {
    /// `model` is `"provider/model"` (for example `azure_openai/gpt-4.1-mini`).
    /// `Err` refuses the run with that user-facing message.
    async fn check_model(&self, workspace_id: Option<&str>, model: &str) -> Result<(), String>;
}
