//! distri's A2A extensions.
//!
//! `distri-a2a` speaks plain A2A 1.0. Everything distri adds on top — client
//! tools, durable runs, the full event stream, hooks, … — is an A2A
//! extension: declared on the agent card under `capabilities.extensions`
//! (`https://distri.dev/a2a/extensions/<name>/v1`, never `required`), and used
//! only when the client activates it, with the `A2A-Extensions` header
//! (comma-separated or repeated) or `Message.extensions`. The server echoes
//! the activated set in the `A2A-Extensions` response header.
//!
//! Each extension has a doc under `docs/a2a-extensions/<name>.md`.
//!
//! Compatibility (one release): a request that calls a 0.x method name
//! (`message/stream`, …) activates every distri extension implicitly, and a
//! 1.0 request that sends distri metadata without activating the extension
//! it belongs to is still honoured, with a deprecation warning in the log.

use std::collections::BTreeSet;

use distri_a2a::{AgentCard, AgentExtension, Metadata};
use serde_json::{Value, json};

/// `Part.metadata` key that names a distri part kind (`tool_call`,
/// `tool_result`, `artifact`, `resource_link`, …) carried as a `DataPart`.
pub use distri_a2a::legacy::PART_TYPE_KEY;

/// Prefix of every distri extension URI.
pub const EXTENSION_URI_PREFIX: &str = "https://distri.dev/a2a/extensions/";

/// A distri A2A extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DistriExtension {
    /// Tools the client runs: `external_tools` metadata, tool call / result
    /// parts, `tool_*` events, `complete-tool`.
    ClientTools,
    /// Runs outlive the process; client tool calls are pending calls that are
    /// claimed. Requires `client-tools`.
    DurableRuns,
    /// Every `status-update.metadata` carries the distri event envelope.
    EventStream,
    /// Inline hooks and dynamic prompt values.
    Hooks,
    /// Per-request run configuration (`definition_overrides`, `env_vars`, …).
    RunConfig,
    /// Todos, context budget and compaction.
    Context,
    /// MCP-Apps `ui://` resources.
    McpApps,
    /// Browser sessions and live view.
    Browser,
    /// Tags, trace context, LLM-call and diagnostic events.
    Observability,
    /// Channel (Telegram, Slack, …) replies.
    Channels,
}

impl DistriExtension {
    pub const ALL: [DistriExtension; 10] = [
        DistriExtension::ClientTools,
        DistriExtension::DurableRuns,
        DistriExtension::EventStream,
        DistriExtension::Hooks,
        DistriExtension::RunConfig,
        DistriExtension::Context,
        DistriExtension::McpApps,
        DistriExtension::Browser,
        DistriExtension::Observability,
        DistriExtension::Channels,
    ];

    /// Short name (`client-tools`, …), also the doc file name.
    pub fn name(self) -> &'static str {
        match self {
            DistriExtension::ClientTools => "client-tools",
            DistriExtension::DurableRuns => "durable-runs",
            DistriExtension::EventStream => "event-stream",
            DistriExtension::Hooks => "hooks",
            DistriExtension::RunConfig => "run-config",
            DistriExtension::Context => "context",
            DistriExtension::McpApps => "mcp-apps",
            DistriExtension::Browser => "browser",
            DistriExtension::Observability => "observability",
            DistriExtension::Channels => "channels",
        }
    }

    /// `https://distri.dev/a2a/extensions/<name>/v1`.
    pub fn uri(self) -> &'static str {
        match self {
            DistriExtension::ClientTools => "https://distri.dev/a2a/extensions/client-tools/v1",
            DistriExtension::DurableRuns => "https://distri.dev/a2a/extensions/durable-runs/v1",
            DistriExtension::EventStream => "https://distri.dev/a2a/extensions/event-stream/v1",
            DistriExtension::Hooks => "https://distri.dev/a2a/extensions/hooks/v1",
            DistriExtension::RunConfig => "https://distri.dev/a2a/extensions/run-config/v1",
            DistriExtension::Context => "https://distri.dev/a2a/extensions/context/v1",
            DistriExtension::McpApps => "https://distri.dev/a2a/extensions/mcp-apps/v1",
            DistriExtension::Browser => "https://distri.dev/a2a/extensions/browser/v1",
            DistriExtension::Observability => "https://distri.dev/a2a/extensions/observability/v1",
            DistriExtension::Channels => "https://distri.dev/a2a/extensions/channels/v1",
        }
    }

    /// By URI or short name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|e| e.uri() == s || e.name() == s)
    }

    /// Extensions this one needs active too.
    pub fn requires(self) -> &'static [DistriExtension] {
        match self {
            DistriExtension::DurableRuns => &[DistriExtension::ClientTools],
            _ => &[],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            DistriExtension::ClientTools => {
                "Client tools: the client declares tools it runs (metadata external_tools, \
                 runtime_mode, tool_metadata); tool calls and results travel as DataParts with \
                 metadata[\"distri/part_type\"]; results return via complete-tool or a message."
            }
            DistriExtension::DurableRuns => {
                "Durable runs: runs outlive the process. Client tool calls are pending calls \
                 a client lists again after a reload and claims before running. Requires \
                 client-tools."
            }
            DistriExtension::EventStream => {
                "Event stream: every status-update.metadata carries the distri event envelope \
                 (type, agent_id, parent_task_id, step_id, attempt, final on the root's end); \
                 text streams as text_message_* events."
            }
            DistriExtension::Hooks => {
                "Hooks: inline_hook_requested events answered at /event/hooks; metadata \
                 dynamic_values / dynamic_sections."
            }
            DistriExtension::RunConfig => {
                "Run config: per-request metadata definition_overrides, load_skills, env_vars, \
                 dry_run, is_sandbox."
            }
            DistriExtension::Context => {
                "Context: todos_updated, context_budget_update, context_compaction, \
                 compaction_requested events; POST /tasks/{id}/compact."
            }
            DistriExtension::McpApps => {
                "MCP Apps: ui:// resource-link parts and their read/call endpoints."
            }
            DistriExtension::Browser => {
                "Browser: metadata browser_session_id; browser_session_started and live_view \
                 events; POST /browser/session."
            }
            DistriExtension::Observability => {
                "Observability: metadata tags, trace_context; llm_call and diagnostic_log events."
            }
            DistriExtension::Channels => "Channels: channel_reply events.",
        }
    }

    /// `AgentExtension.params`: what the extension adds, by kind.
    pub fn params(self) -> Metadata {
        let v = match self {
            DistriExtension::ClientTools => json!({
                "metadata": ["external_tools", "runtime_mode", "tool_metadata"],
                "part_metadata_key": PART_TYPE_KEY,
                "part_types": ["tool_call", "tool_result"],
                "events": ["tool_calls", "tool_results", "tool_execution_start", "tool_execution_end"],
                "complete_tool": "/v1/agents/{agent_id}/complete-tool",
                "well_known_tools": ["ask_user", "approval"],
            }),
            DistriExtension::DurableRuns => json!({
                "requires": [DistriExtension::ClientTools.uri()],
                "pending_calls": "/v1/tasks/{task_id}/pending-calls",
                "thread_pending_calls": "/v1/threads/{thread_id}/pending-calls",
                "claim": "/v1/pending-calls/{id}/claim",
                "events": ["run_parked"],
                "envelope": ["attempt"],
            }),
            DistriExtension::EventStream => json!({
                "envelope": ["type", "agent_id", "parent_task_id", "parent_tool_call_id",
                             "title", "agent_type", "step_id", "attempt", "final"],
                "events": ["run_started", "run_finished", "run_error", "plan_started",
                           "plan_finished", "plan_pruned", "step_started", "step_completed",
                           "reflect_started", "reflect_finished", "text_message_start",
                           "text_message_content", "text_message_end", "agent_handover"],
                "replay": "/v1/tasks/{task_id}/events",
            }),
            DistriExtension::Hooks => json!({
                "metadata": ["dynamic_values", "dynamic_sections"],
                "events": ["inline_hook_requested"],
                "complete_hook": "/v1/event/hooks",
            }),
            DistriExtension::RunConfig => json!({
                "metadata": ["definition_overrides", "load_skills", "env_vars", "dry_run",
                             "is_sandbox", "additional_attributes"],
            }),
            DistriExtension::Context => json!({
                "events": ["todos_updated", "context_budget_update", "context_compaction",
                           "compaction_requested"],
                "compact": "/v1/tasks/{task_id}/compact",
            }),
            DistriExtension::McpApps => json!({
                "part_types": ["resource_link"],
                "resources_read": "/v1/mcp-apps/resources/read",
                "tools_call": "/v1/mcp-apps/tools/call",
                "see": "io.modelcontextprotocol/ui",
            }),
            DistriExtension::Browser => json!({
                "metadata": ["browser_session_id"],
                "events": ["browser_session_started", "live_view"],
                "session": "/v1/browser/session",
            }),
            DistriExtension::Observability => json!({
                "metadata": ["tags", "trace_context"],
                "events": ["llm_call", "diagnostic_log"],
            }),
            DistriExtension::Channels => json!({ "events": ["channel_reply"] }),
        };
        match v {
            Value::Object(m) => m,
            _ => Metadata::new(),
        }
    }

    /// The card entry (`required: false`).
    pub fn agent_extension(self) -> AgentExtension {
        AgentExtension {
            uri: self.uri().to_string(),
            description: Some(self.description().to_string()),
            required: Some(false),
            params: Some(self.params()),
        }
    }

    /// The extension an `AgentEventType` wire `type` belongs to, beyond the
    /// standard stream (None: the event maps to standard A2A in any mode).
    pub fn for_event_type(event_type: &str) -> Option<Self> {
        Some(match event_type {
            "tool_calls" | "tool_results" | "tool_execution_start" | "tool_execution_end" => {
                DistriExtension::ClientTools
            }
            "run_parked" => DistriExtension::DurableRuns,
            "inline_hook_requested" => DistriExtension::Hooks,
            "todos_updated"
            | "context_budget_update"
            | "context_compaction"
            | "compaction_requested" => DistriExtension::Context,
            "browser_session_started" | "live_view" => DistriExtension::Browser,
            "llm_call" | "diagnostic_log" => DistriExtension::Observability,
            "channel_reply" => DistriExtension::Channels,
            _ => DistriExtension::EventStream,
        })
    }

    /// The extension a request-metadata key belongs to.
    pub fn for_metadata_key(key: &str) -> Option<Self> {
        Some(match key {
            "external_tools" | "runtime_mode" | "tool_metadata" => DistriExtension::ClientTools,
            "dynamic_values" | "dynamic_sections" => DistriExtension::Hooks,
            "definition_overrides"
            | "load_skills"
            | "env_vars"
            | "dry_run"
            | "is_sandbox"
            | "additional_attributes" => DistriExtension::RunConfig,
            "browser_session_id" => DistriExtension::Browser,
            "tags" | "trace_context" => DistriExtension::Observability,
            _ => return None,
        })
    }
}

/// The extensions a server declares on its cards. `durable_runs` follows the
/// caller's workspace (a host can switch durable runs per workspace).
pub fn card_extensions(durable_runs: bool) -> Vec<AgentExtension> {
    DistriExtension::ALL
        .into_iter()
        .filter(|e| durable_runs || *e != DistriExtension::DurableRuns)
        .map(DistriExtension::agent_extension)
        .collect()
}

/// Put distri's extensions on a card (replacing any distri protocol
/// extension entries; the `agent` descriptor and others are kept).
pub fn set_card_extensions(card: &mut AgentCard, durable_runs: bool) {
    card.capabilities
        .extensions
        .retain(|e| DistriExtension::parse(&e.uri).is_none());
    card.capabilities
        .extensions
        .extend(card_extensions(durable_runs));
}

/// The `agent` extension: a declarative descriptor of how to use the agent
/// (not activated by clients). The card is the public descriptor; the
/// definition (instructions, tools, model, MCP, connections) is a developer
/// API and never on the card.
pub const AGENT_EXTENSION_URI: &str = "https://distri.dev/a2a/extensions/agent/v1";

/// `params` of the `agent` extension.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct AgentDescriptor {
    /// `standard_agent` or `workflow_agent`.
    pub agent_type: String,
    /// External (client) tool names the agent expects (`"*"` excluded).
    pub external_tools: Vec<String>,
    /// The agent accepts any client tool (`tools.external = ["*"]`).
    pub any_tool: bool,
    /// `web`, `server` or `cli`; absent when the definition sets none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// The agent streams a browser session.
    pub browser_streaming: bool,
    /// The saved version number (hosts that version agents set it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_no: Option<i32>,
}

impl AgentDescriptor {
    /// From an agent config (same contract logic as version checks).
    pub fn of(config: &crate::configuration::AgentConfig, version_no: Option<i32>) -> Self {
        let value = serde_json::to_value(config).unwrap_or_default();
        let contract = crate::agent_contract::contract_of(&value);
        let (agent_type, browser_streaming) = match config {
            crate::configuration::AgentConfig::StandardAgent(def) => {
                ("standard_agent", def.should_use_browser())
            }
            crate::configuration::AgentConfig::WorkflowAgent(_) => ("workflow_agent", false),
        };
        AgentDescriptor {
            agent_type: agent_type.to_string(),
            external_tools: contract.external_tools.into_iter().collect(),
            any_tool: crate::agent_contract::accepts_any_tool(&value),
            runtime: contract.runtime,
            browser_streaming,
            version_no,
        }
    }

    /// The card entry.
    pub fn agent_extension(&self) -> AgentExtension {
        AgentExtension {
            uri: AGENT_EXTENSION_URI.to_string(),
            description: Some(
                "Agent descriptor: what a client needs to use this agent (type, the client \
                 tools it expects, its environment)."
                    .to_string(),
            ),
            required: Some(false),
            params: match serde_json::to_value(self) {
                Ok(Value::Object(m)) => Some(m),
                _ => None,
            },
        }
    }

    /// Read it back from a card.
    pub fn from_card(card: &AgentCard) -> Option<Self> {
        let ext = card
            .capabilities
            .extensions
            .iter()
            .find(|e| e.uri == AGENT_EXTENSION_URI)?;
        serde_json::from_value(Value::Object(ext.params.clone()?)).ok()
    }
}

/// Put (or replace) the `agent` descriptor on a card.
pub fn set_agent_extension(card: &mut AgentCard, descriptor: &AgentDescriptor) {
    card.capabilities
        .extensions
        .retain(|e| e.uri != AGENT_EXTENSION_URI);
    card.capabilities
        .extensions
        .push(descriptor.agent_extension());
}

/// How a request came to have its extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivationSource {
    /// `A2A-Extensions` header and/or `Message.extensions`.
    #[default]
    Explicit,
    /// A 0.x method name: every distri extension, implicitly (deprecated).
    Legacy,
}

/// The extensions a request activated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveExtensions {
    distri: BTreeSet<DistriExtension>,
    /// Activated URIs that are not distri's (kept for the echo).
    other: Vec<String>,
    pub source: ActivationSource,
}

impl ActiveExtensions {
    /// From the `A2A-Extensions` header values and `Message.extensions`.
    pub fn from_request<'a>(
        header_values: impl IntoIterator<Item = &'a str>,
        message_extensions: &[String],
    ) -> Self {
        let mut uris = distri_a2a::parse_extensions_header(header_values);
        for u in message_extensions {
            if !uris.contains(u) {
                uris.push(u.clone());
            }
        }
        let mut out = ActiveExtensions::default();
        for uri in uris {
            match DistriExtension::parse(&uri) {
                Some(e) if uri.starts_with(EXTENSION_URI_PREFIX) => {
                    out.distri.insert(e);
                }
                _ => out.other.push(uri),
            }
        }
        out
    }

    /// Every distri extension (0.x callers, the CLI before activation).
    pub fn all_legacy() -> Self {
        ActiveExtensions {
            distri: DistriExtension::ALL.into_iter().collect(),
            other: Vec::new(),
            source: ActivationSource::Legacy,
        }
    }

    pub fn with(mut self, ext: DistriExtension) -> Self {
        self.distri.insert(ext);
        self
    }

    pub fn is_active(&self, ext: DistriExtension) -> bool {
        self.distri.contains(&ext)
    }

    pub fn is_legacy(&self) -> bool {
        self.source == ActivationSource::Legacy
    }

    /// Activated distri extensions.
    pub fn distri(&self) -> impl Iterator<Item = DistriExtension> + '_ {
        self.distri.iter().copied()
    }

    /// URIs to echo in the `A2A-Extensions` response header: the activated
    /// extensions the server supports (`supported` = the card's URIs).
    pub fn echo(&self, supported: &[String]) -> Vec<String> {
        self.distri
            .iter()
            .map(|e| e.uri().to_string())
            .chain(self.other.iter().cloned())
            .filter(|u| supported.iter().any(|s| s == u))
            .collect()
    }

    /// Activated URIs (distri and other), for `-32008` checks.
    pub fn uris(&self) -> Vec<String> {
        self.distri
            .iter()
            .map(|e| e.uri().to_string())
            .chain(self.other.iter().cloned())
            .collect()
    }

    /// Header value (`A2A-Extensions`) for these extensions.
    pub fn header_value(&self) -> String {
        self.uris().join(", ")
    }

    /// The first extension the card marks `required` that this request did
    /// not activate (refuse with `-32008`).
    pub fn missing_required<'c>(&self, card_extensions: &'c [AgentExtension]) -> Option<&'c str> {
        let uris = self.uris();
        card_extensions
            .iter()
            .find(|e| e.required == Some(true) && !uris.iter().any(|u| *u == e.uri))
            .map(|e| e.uri.as_str())
    }

    /// Distri request-metadata keys used without their extension active.
    /// Returns the extensions they belong to (deduplicated). Callers log a
    /// deprecation and honour them for one release.
    pub fn unactivated_metadata(&self, metadata: Option<&Metadata>) -> Vec<DistriExtension> {
        let mut out = BTreeSet::new();
        if let Some(m) = metadata {
            for key in m.keys() {
                if let Some(ext) = DistriExtension::for_metadata_key(key) {
                    if !self.is_active(ext) {
                        out.insert(ext);
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_and_names_round_trip() {
        for e in DistriExtension::ALL {
            assert_eq!(DistriExtension::parse(e.uri()), Some(e));
            assert_eq!(DistriExtension::parse(e.name()), Some(e));
            assert_eq!(
                e.uri(),
                format!("{EXTENSION_URI_PREFIX}{}/v1", e.name()),
                "uri pattern"
            );
            let ext = e.agent_extension();
            assert_eq!(ext.required, Some(false));
            assert!(ext.params.is_some_and(|p| !p.is_empty()));
        }
    }

    #[test]
    fn activation_from_header_and_message() {
        let a = ActiveExtensions::from_request(
            [
                "https://distri.dev/a2a/extensions/event-stream/v1, urn:other",
                "https://distri.dev/a2a/extensions/client-tools/v1",
            ],
            &["https://distri.dev/a2a/extensions/durable-runs/v1".to_string()],
        );
        assert!(a.is_active(DistriExtension::EventStream));
        assert!(a.is_active(DistriExtension::ClientTools));
        assert!(a.is_active(DistriExtension::DurableRuns));
        assert!(!a.is_active(DistriExtension::Hooks));
        assert!(!a.is_legacy());
        let card: Vec<String> = card_extensions(true).into_iter().map(|e| e.uri).collect();
        let echo = a.echo(&card);
        assert_eq!(echo.len(), 3, "urn:other is not supported, not echoed");
    }

    #[test]
    fn missing_required_extension() {
        let mut exts = card_extensions(false);
        exts.push(AgentExtension {
            uri: "urn:must".into(),
            required: Some(true),
            ..Default::default()
        });
        let none = ActiveExtensions::default();
        assert_eq!(none.missing_required(&exts), Some("urn:must"));
        let ok = ActiveExtensions::from_request(["urn:must"], &[]);
        assert_eq!(ok.missing_required(&exts), None);
    }

    #[test]
    fn card_extensions_follow_durable() {
        assert!(
            card_extensions(true)
                .iter()
                .any(|e| e.uri == DistriExtension::DurableRuns.uri())
        );
        assert!(
            !card_extensions(false)
                .iter()
                .any(|e| e.uri == DistriExtension::DurableRuns.uri())
        );
    }

    #[test]
    fn unactivated_metadata_is_reported() {
        let m: Metadata = serde_json::from_value(json!({
            "external_tools": [], "env_vars": {}, "something": 1
        }))
        .unwrap();
        let a = ActiveExtensions::default().with(DistriExtension::ClientTools);
        assert_eq!(
            a.unactivated_metadata(Some(&m)),
            vec![DistriExtension::RunConfig]
        );
    }
}
