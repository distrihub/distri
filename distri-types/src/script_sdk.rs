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

/// The most calls one run token (one step's script) may make.
pub const MAX_CALLS_PER_TOKEN: usize = 200;
/// The most `agent` calls one script may make.
pub const MAX_AGENT_CALLS_PER_TOKEN: usize = 10;
/// The most `notify` calls one script may make.
pub const MAX_NOTIFY_CALLS_PER_TOKEN: usize = 20;
/// The most characters of an error a record or a response carries.
pub const MAX_ERROR_CHARS: usize = 300;
/// Callback-data prefixes a script's buttons may not use: the bot's run-operation buttons.
pub const RESERVED_CALLBACK_PREFIXES: &[&str] = &["wf:op:", "wfo:"];

/// An error as a record or a response shows it: token-like values redacted, at most
/// [`MAX_ERROR_CHARS`] characters.
pub fn clean_error(text: &str) -> String {
    let scrubbed = crate::memory::guard::scrub_secrets(text.trim());
    if scrubbed.chars().count() <= MAX_ERROR_CHARS {
        return scrubbed;
    }
    let cut: String = scrubbed.chars().take(MAX_ERROR_CHARS - 1).collect();
    format!("{cut}…")
}

/// The per-op cap for `op`, when it has one below [`MAX_CALLS_PER_TOKEN`].
pub fn op_cap(op: &str) -> Option<usize> {
    match op {
        "agent" => Some(MAX_AGENT_CALLS_PER_TOKEN),
        "notify" => Some(MAX_NOTIFY_CALLS_PER_TOKEN),
        _ => None,
    }
}

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
    /// `host:port` of [`Self::url`], for `--allow-net`: always with the port (the scheme's
    /// default when the URL has none), so Deno allows that one port and not every port of the
    /// host.
    pub fn net_host(&self) -> Option<String> {
        let (scheme, rest) = self.url.split_once("://").unwrap_or(("https", &self.url));
        let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
        if host.is_empty() {
            return None;
        }
        let has_port = match host.rsplit_once(':') {
            // `[::1]:8080` has a port; `[::1]` does not.
            Some((h, p)) => !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !h.is_empty(),
            None => false,
        };
        if has_port {
            return Some(host.to_string());
        }
        let port = if scheme.eq_ignore_ascii_case("http") { 80 } else { 443 };
        Some(format!("{host}:{port}"))
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
    /// The script made more calls than its step allows (see [`MAX_CALLS_PER_TOKEN`]).
    #[error("too many calls: {0}")]
    TooMany(String),
}

impl ScriptCallError {
    pub fn code(&self) -> &'static str {
        match self {
            ScriptCallError::Unauthorized => "unauthorized",
            ScriptCallError::Invalid(_) => "invalid",
            ScriptCallError::Failed(_) => "failed",
            ScriptCallError::TooMany(_) => "too_many_calls",
        }
    }
    /// The HTTP status a host answers with.
    pub fn http_status(&self) -> u16 {
        match self {
            ScriptCallError::Unauthorized => 401,
            ScriptCallError::Invalid(_) => 422,
            ScriptCallError::Failed(_) => 200,
            ScriptCallError::TooMany(_) => 429,
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
        assert_eq!(a.net_host().as_deref(), Some("api.distri.dev:443"));
        assert!(!format!("{a:?}").contains("dsr_secret"));
        let b = CodeSdkAccess {
            url: "http://127.0.0.1:8080/v1/workflows/sdk".into(),
            token: String::new(),
        };
        assert_eq!(b.net_host().as_deref(), Some("127.0.0.1:8080"));
        let c = CodeSdkAccess {
            url: "http://localhost/v1/workflows/sdk".into(),
            token: String::new(),
        };
        assert_eq!(c.net_host().as_deref(), Some("localhost:80"));
    }

    #[test]
    fn errors_are_scrubbed_and_cut() {
        let e = clean_error("upstream said: Bearer abcdefghijklmnopqrstuvwxyz0123456789 refused");
        assert!(!e.contains("abcdefghij"), "{e}");
        assert!(e.contains("[redacted]"), "{e}");
        let long = clean_error(&"x".repeat(1000));
        assert_eq!(long.chars().count(), MAX_ERROR_CHARS);
        assert_eq!(ScriptCallError::TooMany("x".into()).http_status(), 429);
        assert_eq!(op_cap("notify"), Some(MAX_NOTIFY_CALLS_PER_TOKEN));
    }
}
