pub mod a2a;
pub mod agent;
pub mod broadcast;
pub mod connections;

// Re-export from distri-types so callers can write `distri_core::ApiError`.
pub use distri_types::{ApiError, ApiResult};
pub mod worker;

pub mod claude_llm;
pub mod llm;
pub mod llm_service;
pub mod logging;

pub mod openai_responses_llm;
pub mod secrets;

// Re-export modules moved to llm-gateway
pub use llm_gateway::claude_client;
pub use llm_gateway::gateway_config;
pub use llm_gateway::openai_responses_client;
pub use llm_gateway::provider_config;
pub mod servers;
pub mod tools;
pub mod types {
    pub use distri_types::*;
}
pub mod hooks;
mod hooks_runtime;
pub use distri_auth::UserContext;
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
