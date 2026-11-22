use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent-level browser flag. The browser itself is the internal `browser` MCP
/// app (obscura in the thread's sandbox), which agents opt into with
/// `[[tools.mcp]] server = "browser"`; this only marks an agent as wanting it.
///
/// Unknown keys are ignored so stored agent files that still carry the old
/// runtime overrides from the old browser driver (`headless`, ...) keep loading.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct BrowserAgentConfig {
    /// Whether the orchestrator should offer the browser agent
    pub enabled: bool,
    /// Persist and restore session cookies/state between runs
    pub persist_session: bool,
}

impl BrowserAgentConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn should_persist_session(&self) -> bool {
        self.persist_session
    }
}
