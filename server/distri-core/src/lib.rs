pub mod a2a;
pub mod agent;
pub mod gateway_config;
pub mod llm;
pub mod llm_service;
pub mod logging;
pub mod secrets;
pub mod servers;
pub mod tools;
pub mod types {
    pub use distri_types::*;
}
pub mod hooks;
mod hooks_runtime;
pub mod voice;
pub mod workflow;

pub use distri_auth::{
    auth_routes, CallbackConfig, CliAuthServer, ToolAuthRequestContext, UserContext,
};
pub use distri_stores::*;
pub use distri_types::AgentError;
pub use types::{McpDefinition, McpSession, ModelSettings, StandardDefinition};

#[cfg(test)]
mod tests;

#[cfg(test)]
pub use logging::init_logging;

pub use agent::orchestrator::{AgentOrchestrator, AgentOrchestratorBuilder};
pub use hooks_runtime::HookRegistry;
pub use secrets::{ResolvedSecret, SecretResolver, SecretSource};
