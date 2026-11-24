//! The script SDK (`@distri/runtime`): what a workflow `code` step may ask of its run, and what
//! the run records about it (distrihub/distri-cloud#304, phase 3).
//!
//! A `code` step runs in a sandbox. When the host offers the SDK, the step gets a short-lived
//! **run token** bound to its workspace, run, step and attempt; the script calls back to the host
//! (`POST {url}/call`, `Authorization: Bearer <run token>`) with a [`ScriptCall`], and the host
//! runs it *as the run*: tools get their connection tokens on the host, never in the script.
//! Every call is kept on the step as a [`ScriptCallRecord`] so its side effects are visible.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Prefix of a run token.
pub const RUN_TOKEN_PREFIX: &str = "dsr_";

/// Where a sandboxed script reaches its run, and with what token. Passed to the code runner in
/// [`crate::CodeRunRequest::sdk`]; the runner adds the URL's host to the script's net allow-list.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeSdkAccess {
    /// The SDK base URL (`…/v1/workflows/sdk`); calls go to `{url}/call`.
    pub url: String,
    /// The run token (`dsr_…`). Never logged.
    pub token: String,
}

impl std::fmt::Debug for CodeSdkAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeSdkAccess")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl CodeSdkAccess {
    /// `host` or `host:port` of [`Self::url`], for `--allow-net`.
    pub fn net_host(&self) -> Option<String> {
        let rest = self
            .url
            .split_once("://")
            .map(|(_, r)| r)
            .unwrap_or(&self.url);
        let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
        (!host.is_empty()).then(|| host.to_string())
    }
}

/// One call from a script to its run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ScriptCall {
    /// Send the owner a message (`buttons`: rows of `ChannelButton`).
    Notify {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        buttons: Vec<Vec<crate::channel_commands::ChannelButton>>,
    },
    /// The workflow's state document.
    StateGet {},
    /// Shallow-merge `patch` into the workflow's state (`null` removes a key).
    StateSet { patch: serde_json::Value },
    /// Run one of the run's tools on the host.
    Tool {
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    /// Run an agent (a child task of the run) with `prompt`.
    Agent { agent: String, prompt: String },
    /// Record a memory signal (the `memory` step's signal fields).
    MemorySignal {
        #[serde(default)]
        signal: serde_json::Value,
    },
}

impl ScriptCall {
    pub fn op(&self) -> &'static str {
        match self {
            ScriptCall::Notify { .. } => "notify",
            ScriptCall::StateGet {} => "state_get",
            ScriptCall::StateSet { .. } => "state_set",
            ScriptCall::Tool { .. } => "tool",
            ScriptCall::Agent { .. } => "agent",
            ScriptCall::MemorySignal { .. } => "memory_signal",
        }
    }

    /// Whether the call changes something outside the run's own state (a message, a tool, an
    /// agent run, a memory write). A step whose script made one is not replay-safe.
    pub fn side_effect(&self) -> bool {
        !matches!(self, ScriptCall::StateGet {} | ScriptCall::StateSet { .. })
    }

    /// A short, secret-free description for the step's record.
    pub fn summary(&self) -> String {
        fn cut(s: &str, n: usize) -> String {
            if s.chars().count() <= n {
                s.to_string()
            } else {
                format!("{}…", s.chars().take(n).collect::<String>())
            }
        }
        match self {
            ScriptCall::Notify { text, buttons } => {
                let n: usize = buttons.iter().map(|r| r.len()).sum();
                if n > 0 {
                    format!("{} ({n} buttons)", cut(text, 160))
                } else {
                    cut(text, 160)
                }
            }
            ScriptCall::StateGet {} => "state".to_string(),
            ScriptCall::StateSet { patch } => {
                let keys: Vec<String> = patch
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                format!("set {}", keys.join(", "))
            }
            ScriptCall::Tool { name, .. } => name.clone(),
            ScriptCall::Agent { agent, prompt } => format!("{agent}: {}", cut(prompt, 120)),
            ScriptCall::MemorySignal { signal } => signal
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("signal")
                .to_string(),
        }
    }
}

/// What a step keeps about one script call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptCallRecord {
    /// 1, 2, … in the order the calls arrived.
    pub seq: u32,
    pub op: String,
    pub at: DateTime<Utc>,
    pub ok: bool,
    /// Whether it changed something outside the run's state (see [`ScriptCall::side_effect`]).
    pub side_effect: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Whether any of `calls` had a side effect.
pub fn any_side_effect(calls: &[ScriptCallRecord]) -> bool {
    calls.iter().any(|c| c.side_effect)
}

/// Why a script call was refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptCallError {
    /// No such token, or it expired or was revoked (the step ended).
    #[error("unauthorized: the run token is unknown, expired or revoked")]
    Unauthorized,
    /// The call is malformed.
    #[error("invalid: {0}")]
    Invalid(String),
    /// The call ran and failed.
    #[error("{0}")]
    Failed(String),
}

impl ScriptCallError {
    pub fn code(&self) -> &'static str {
        match self {
            ScriptCallError::Unauthorized => "unauthorized",
            ScriptCallError::Invalid(_) => "invalid",
            ScriptCallError::Failed(_) => "failed",
        }
    }
    /// The HTTP status a host answers with.
    pub fn http_status(&self) -> u16 {
        match self {
            ScriptCallError::Unauthorized => 401,
            ScriptCallError::Invalid(_) => 422,
            ScriptCallError::Failed(_) => 200,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_parse_by_op() {
        let c: ScriptCall =
            serde_json::from_value(serde_json::json!({"op": "state_set", "patch": {"a": 1}}))
                .unwrap();
        assert_eq!(c.op(), "state_set");
        assert!(!c.side_effect());
        let c: ScriptCall =
            serde_json::from_value(serde_json::json!({"op": "notify", "text": "hi"})).unwrap();
        assert!(c.side_effect());
        assert_eq!(c.summary(), "hi");
        let c: ScriptCall = serde_json::from_value(serde_json::json!({"op": "state_get"})).unwrap();
        assert_eq!(c, ScriptCall::StateGet {});
        assert!(serde_json::from_value::<ScriptCall>(serde_json::json!({"op": "rm_rf"})).is_err());
    }

    #[test]
    fn net_host_and_redacted_debug() {
        let a = CodeSdkAccess {
            url: "https://api.distri.dev/v1/workflows/sdk".into(),
            token: "dsr_secret".into(),
        };
        assert_eq!(a.net_host().as_deref(), Some("api.distri.dev"));
        assert!(!format!("{a:?}").contains("dsr_secret"));
        let b = CodeSdkAccess {
            url: "http://127.0.0.1:8080/v1/workflows/sdk".into(),
            token: String::new(),
        };
        assert_eq!(b.net_host().as_deref(), Some("127.0.0.1:8080"));
    }
}
