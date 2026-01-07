pub use browsr_types::BrowsrClientConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent-level configuration for enabling the shared browser runtime
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserAgentConfig {
    /// Whether the orchestrator should eagerly initialize the browser
    pub enabled: bool,
    /// Persist and restore session cookies/state between runs
    pub persist_session: bool,
    /// Optional runtime overrides for the Chromium driver
    #[serde(skip_serializing_if = "Option::is_none", flatten)]
    pub runtime: Option<BrowsrClientConfig>,
}

impl Default for BrowserAgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            persist_session: false,
            runtime: None,
        }
    }
}

impl BrowserAgentConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn should_persist_session(&self) -> bool {
        self.persist_session
    }

    pub fn runtime_config(&self) -> BrowsrClientConfig {
        self.runtime.clone().unwrap_or_default()
    }
}
