use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{MessageRole, ToolCall, ToolResponse};
use crate::execution::ContextBudget;
use crate::hooks::InlineHookRequest;

/// Token usage information for a run
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RunUsage {
    /// Actual tokens used (from LLM response)
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens read from provider cache (e.g., Anthropic prompt caching)
    #[serde(default)]
    pub cached_tokens: u32,
    /// Estimated tokens (pre-call estimate)
    pub estimated_tokens: u32,
    /// Model used for this run (e.g., "gpt-5.1", "claude-sonnet-4")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Estimated cost in USD (based on model pricing)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// What a child run says about itself on every frame it emits, so a consumer
/// can place it under the tool call that started it and label it. All fields
/// are absent on root runs.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct ChildRun {
    /// `tool_call_id` of the `invoke_agent` call (in the parent's turn) that
    /// started this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    /// Short human label for the run, set by the caller (`invoke_agent`'s
    /// optional `title`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Registered agent name for a named target, `"adhoc"` for an ad-hoc worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

impl ChildRun {
    pub fn is_empty(&self) -> bool {
        self.parent_tool_call_id.is_none() && self.title.is_none() && self.agent_type.is_none()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct AgentEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub thread_id: String,
    pub run_id: String,
    pub event: AgentEventType,
    pub task_id: String,
    /// Ancestor task that dispatched this run. `None` for root tasks.
    /// Lets consumers reconstruct the task tree from the event stream
    /// (and route sub-agent events to the right node in the FE store).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    /// Set on child runs: the call that started them, their title and type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<ChildRun>,
    pub agent_id: String,
    /// User ID for usage tracking
    #[serde(default)]
    pub user_id: Option<String>,
    /// Identifier ID for tenant/project-level usage tracking
    #[serde(default)]
    pub identifier_id: Option<String>,
    /// Workspace ID for workspace-scoped usage tracking
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Channel ID for channel-scoped usage tracking
    #[serde(default)]
    pub channel_id: Option<String>,
}

impl AgentEvent {
    /// Reconstruct an AgentEvent from a stored TaskEvent (e.g. for history replay).
    pub fn from_task_event(task_event: &crate::TaskEvent, thread_id: &str) -> Self {
        Self {
            event: task_event.event.clone(),
            agent_id: String::new(),
            timestamp: chrono::DateTime::from_timestamp_millis(task_event.created_at)
                .unwrap_or_default(),
            thread_id: thread_id.to_string(),
            run_id: String::new(),
            task_id: String::new(),
            parent_task_id: None,
            child: None,
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }
}

/// Typed payload that goes into the A2A `TaskStatusUpdateEvent.metadata` field
/// for every event the server emits. Carries the routing fields the wire
/// envelope (A2A) doesn't model — `parent_task_id` (for the FE/CLI task tree)
/// and `agent_id` (for tool-registry lookups on sub-agent events).
///
/// Use `from_event` / `to_agent_event` to round-trip without loose-JSON
/// extraction. The A2A `TaskStatusUpdateEvent` itself is not extended —
/// everything Distri-specific lives inside this typed body.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentEventEnvelope {
    /// The event variant inline (`type = "..."`, plus variant fields).
    /// `serde(flatten)` keeps the wire shape readable.
    #[serde(flatten)]
    pub event: AgentEventType,
    /// Definition name of the agent that emitted this event. For sub-agent
    /// events relayed through a parent's stream, this is the sub-agent's
    /// name — the stream's URL agent_id is the parent's, so consumers need
    /// this to look up tool registries / display names per-event.
    pub agent_id: String,
    /// Dispatching task for sub-agent events. Absent for root-task events.
    /// Lets the consumer route per-task without modifying the A2A spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    /// `parent_tool_call_id`, `title` and `agent_type` of a child run, flat
    /// beside `parent_task_id` on the wire.
    #[serde(flatten, default, skip_serializing_if = "ChildRun::is_empty")]
    pub child: ChildRun,
}

impl AgentEventEnvelope {
    /// Build an envelope from a full `AgentEvent` for serialization.
    pub fn from_event(event: &AgentEvent) -> Self {
        Self {
            event: event.event.clone(),
            agent_id: event.agent_id.clone(),
            parent_task_id: event.parent_task_id.clone(),
            child: event.child.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum AgentEventType {
    /// Verbose diagnostic message streamed from server to client (only emitted when verbose=true).
    DiagnosticLog {
        message: String,
    },

    // Main run events
    RunStarted {},
    RunFinished {
        success: bool,
        total_steps: usize,
        failed_steps: usize,
        /// Token usage for this run
        usage: Option<RunUsage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_budget: Option<ContextBudget>,
    },
    RunError {
        message: String,
        code: Option<String>,
        /// Cumulative token usage at the point of failure
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<RunUsage>,
    },
    PlanStarted {
        initial_plan: bool,
    },
    PlanFinished {
        total_steps: usize,
    },
    PlanPruned {
        removed_steps: Vec<String>,
    },
    // Step execution events
    StepStarted {
        step_id: String,
        step_index: usize,
    },
    StepCompleted {
        step_id: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_budget: Option<ContextBudget>,
        /// Cumulative token usage for this run up to this step
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<RunUsage>,
    },

    // Reflection events (emitted when is_reflection_enabled() and reflection runs)
    ReflectStarted {},
    ReflectFinished {
        should_retry: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    // Tool execution events
    ToolExecutionStart {
        step_id: String,
        tool_call_id: String,
        tool_call_name: String,
        input: Value,
    },
    ToolExecutionEnd {
        step_id: String,
        tool_call_id: String,
        tool_call_name: String,
        success: bool,
    },

    // Message events for streaming
    TextMessageStart {
        message_id: String,
        step_id: String,
        role: MessageRole,
    },
    TextMessageContent {
        message_id: String,
        step_id: String,
        delta: String,
        stripped_content: Option<Vec<(usize, String)>>,
    },
    TextMessageEnd {
        message_id: String,
        step_id: String,
    },

    // Tool call events with parent/child relationships
    ToolCalls {
        step_id: String,
        parent_message_id: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResults {
        step_id: String,
        parent_message_id: Option<String>,
        results: Vec<ToolResponse>,
    },

    // Agent transfer events
    AgentHandover {
        from_agent: String,
        to_agent: String,
        reason: Option<String>,
    },

    /// A live, embeddable view produced by a tool (e.g. browsr viewer, Grafana
    /// dashboard, map widget). The channel renders it inline as an iframe
    /// (web) or as a clickable link (Telegram, WhatsApp, CLI).
    LiveView {
        /// Unique ID for this view — used for updates and teardown
        view_id: String,
        /// URL to embed or link (must be https:// for iframe security)
        url: String,
        /// Human-readable title for the view
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Display mode hint: "inline", "fullscreen", or "pip"
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_mode: Option<String>,
        /// Width hint in pixels
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        /// Height hint in pixels
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
    },

    BrowserSessionStarted {
        session_id: String,
        viewer_url: Option<String>,
        stream_url: Option<String>,
    },

    InlineHookRequested {
        request: InlineHookRequest,
    },

    // TODO events
    TodosUpdated {
        formatted_todos: String,
        action: String,
        todo_count: usize,
        /// Per-call diff: which items got added, had their status
        /// changed, or were removed. Empty when the renderer can't
        /// or didn't compute it (e.g. the very first `write_todos`
        /// call has no prior list to diff against — every item is
        /// `Added`). Renderers should prefer rendering this list
        /// when non-empty and fall back to `formatted_todos`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        changes: Vec<crate::todos::TodoChange>,
    },

    // Context management events
    ContextCompaction {
        tier: CompactionTier,
        tokens_before: usize,
        tokens_after: usize,
        entries_affected: usize,
        context_limit: usize,
        usage_ratio: f64,
        summary: Option<String>,
        /// Skill IDs re-injected after compaction
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        reinjected_skills: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_budget: Option<ContextBudget>,
        /// "auto" when fired by the agent loop's pre-plan trigger,
        /// "manual" when invoked via the `/compact` endpoint or slash command.
        #[serde(default = "default_compaction_source")]
        source: String,
        /// Wall-clock duration of the compaction operation, in milliseconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },

    /// Emitted when a compaction has been requested but before it runs.
    /// Lets observability distinguish manual vs auto triggers.
    CompactionRequested {
        /// "manual" | "auto"
        source: String,
    },

    /// Emitted each turn with the current context budget breakdown.
    ContextBudgetUpdate {
        budget: ContextBudget,
        is_warning: bool,
        is_critical: bool,
    },

    /// A structured channel reply emitted by a workflow `StepKind::Reply`
    /// step. The gateway renders it per channel; non-channel consumers
    /// (CLI, web) render `reply.text` and ignore buttons they can't show.
    ChannelReply {
        reply: crate::channel_commands::ChannelReply,
    },
}

fn default_compaction_source() -> String {
    "auto".to_string()
}

/// Tier of context compaction applied
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTier {
    /// Mechanical: drop old entries, truncate payloads
    Trim,
    /// Semantic: LLM-powered summarization of history
    Summarize,
    /// Emergency: preserve only essentials
    Reset,
}

/// Wire response body for `POST /v1/tasks/{task_id}/compact`. Returned by
/// `Distri::compact_task` and `distrijs`'s `DistriClient.compactTask` —
/// keeping it as a typed struct (instead of `serde_json::Value`) means CLI
/// and SDK callers never have to spell out `.get("tokens_before").and_then(...)`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, utoipa::ToSchema)]
pub struct CompactTaskResponse {
    /// True when entries were modified; false means there was nothing to compact.
    pub compacted: bool,
    /// Free-form explanation for `compacted: false`. Empty / omitted on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Tier applied (only set when `compacted == true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<CompactionTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries_affected: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_ratio: Option<f64>,
}

impl CompactTaskResponse {
    /// Build a `compacted: false` response with a reason.
    pub fn nothing_to_compact(reason: impl Into<String>) -> Self {
        Self {
            compacted: false,
            reason: Some(reason.into()),
            tier: None,
            tokens_before: None,
            tokens_after: None,
            entries_affected: None,
            usage_ratio: None,
        }
    }

    /// Token-count reduction as a percentage (0–100). Returns 0 when
    /// `tokens_before` is missing or zero — handy for surface rendering.
    pub fn reduction_percent(&self) -> f64 {
        match (self.tokens_before, self.tokens_after) {
            (Some(before), Some(after)) if before > 0 => {
                (1.0 - after as f64 / before as f64) * 100.0
            }
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod channel_reply_event_tests {
    use super::*;
    use crate::channel_commands::{ChannelButton, ChannelReply};

    #[test]
    fn channel_reply_event_round_trips() {
        let ev = AgentEventType::ChannelReply {
            reply: ChannelReply {
                text: "Tap to continue:".into(),
                buttons: vec![vec![ChannelButton::WebApp {
                    label: "Continue".into(),
                    url: "https://a.app/lesson/1".into(),
                }]],
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        let back: AgentEventType = serde_json::from_value(v).unwrap();
        assert!(matches!(back, AgentEventType::ChannelReply { .. }));
    }

    #[test]
    fn channel_reply_envelope_round_trips() {
        let ev = AgentEventType::ChannelReply {
            reply: ChannelReply {
                text: "Tap to continue:".into(),
                buttons: vec![vec![ChannelButton::Callback {
                    label: "Continue".into(),
                    callback_data: "wf:open:x".into(),
                }]],
            },
        };
        let agent_event = AgentEvent::new(ev);
        let envelope = AgentEventEnvelope::from_event(&agent_event);
        let v = serde_json::to_value(&envelope).unwrap();
        let back: AgentEventEnvelope = serde_json::from_value(v).expect("envelope deserialize");
        assert!(matches!(back.event, AgentEventType::ChannelReply { .. }));
    }
}

impl AgentEvent {
    pub fn new(event: AgentEventType) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            thread_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            event,
            task_id: uuid::Uuid::new_v4().to_string(),
            parent_task_id: None,
            child: None,
            agent_id: "default".to_string(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    pub fn with_context(
        event: AgentEventType,
        thread_id: String,
        run_id: String,
        task_id: String,
        agent_id: String,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            thread_id,
            run_id,
            task_id,
            parent_task_id: None,
            child: None,
            event,
            agent_id,
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }
}

#[cfg(test)]
mod child_run_tests {
    use super::*;

    fn envelope(child: ChildRun) -> Value {
        serde_json::to_value(AgentEventEnvelope {
            event: AgentEventType::RunStarted {},
            agent_id: "_adhoc_base".into(),
            parent_task_id: Some("parent".into()),
            child,
        })
        .unwrap()
    }

    #[test]
    fn child_fields_sit_flat_beside_parent_task_id() {
        let wire = envelope(ChildRun {
            parent_tool_call_id: Some("call-1".into()),
            title: Some("Newton's first law".into()),
            agent_type: Some("adhoc".into()),
        });
        assert_eq!(wire["type"], "run_started");
        assert_eq!(wire["parent_task_id"], "parent");
        assert_eq!(wire["parent_tool_call_id"], "call-1");
        assert_eq!(wire["title"], "Newton's first law");
        assert_eq!(wire["agent_type"], "adhoc");
    }

    #[test]
    fn root_frames_gain_no_new_keys() {
        let wire = envelope(ChildRun::default());
        for key in ["parent_tool_call_id", "title", "agent_type"] {
            assert!(wire.get(key).is_none(), "{key} leaked onto a root frame");
        }
    }

    #[test]
    fn envelope_round_trips_and_reads_old_payloads() {
        let wire = envelope(ChildRun {
            parent_tool_call_id: Some("call-1".into()),
            title: None,
            agent_type: Some("worker".into()),
        });
        let back: AgentEventEnvelope = serde_json::from_value(wire).unwrap();
        assert_eq!(back.child.parent_tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(back.child.title, None);

        let old = serde_json::json!({"type": "run_started", "agent_id": "a"});
        let back: AgentEventEnvelope = serde_json::from_value(old).unwrap();
        assert!(back.child.is_empty());
    }
}
