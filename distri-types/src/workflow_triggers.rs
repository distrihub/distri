//! Unified workflow trigger taxonomy.
//!
//! Replaces two earlier trigger types that disagreed about shape and
//! scope (`configuration::Trigger` and `channel_commands::ChannelTrigger`
//! — both now deleted). [`WorkflowTrigger`] folds channel triggers
//! (`Slash`/`Callback`/`Message`), agent-level triggers
//! (`Manual`/`Schedule`), and the spec's new variants — `Webhook`,
//! `Event`, `Tool` (workflow exposed as an A2A skill) — into one
//! enum **attached to entry points**. Every trigger either starts a
//! new run or resumes a parked one (event-correlated-by-task —
//! handled through `WorkflowStore.wait_task_id`).

use crate::channels::ChannelProvider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn default_true() -> bool {
    true
}

/// How a workflow run is reached. Lives on an `EntryPoint` (see
/// `distri-workflow`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowTrigger {
    /// Direct API / UI invocation. The implicit default when no
    /// triggers are declared on an entry point.
    Manual,

    /// Cron-based scheduled execution.
    Schedule {
        /// Cron expression, e.g. "0 * * * *" (every hour).
        cron: String,
        /// IANA timezone, e.g. "America/Los_Angeles". Defaults to UTC.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
        #[serde(default = "default_true")]
        enabled: bool,
        /// Default input passed to the workflow on each scheduled run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },

    /// A channel slash command. `args` names positional params.
    Slash {
        name: String,
        #[serde(default)]
        aliases: Vec<String>,
        /// Restrict to these providers; empty = all.
        #[serde(default)]
        channels: Vec<ChannelProvider>,
        #[serde(default)]
        args: Vec<String>,
    },

    /// A channel callback-button tap. `callback_data` is `wf:<id>`
    /// or `wf:<id>:<value>`; `<value>` becomes `input[arg]`.
    Callback {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<String>,
    },

    /// The catch-all for non-command free text on a channel.
    Message {},

    /// Generic inbound HTTP from a 3rd-party service (not a channel
    /// platform's bot webhook). Verified via [`WebhookAuth`].
    Webhook {
        /// URL path suffix mounted at `/v1/workflows/webhook/{path}`.
        path: String,
        /// Methods accepted. Empty = `POST` only.
        #[serde(default)]
        methods: Vec<String>,
        #[serde(default)]
        auth: WebhookAuth,
        #[serde(default)]
        response: WebhookResponse,
    },

    /// Internal event-bus subscription. The workflow starts (or
    /// resumes a parked task waiting on this topic) when a matching
    /// event is published.
    Event {
        topic: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<serde_json::Value>,
    },

    /// The workflow exposed as an A2A skill on the agent card. An
    /// external agent invokes it with `message/send`; the workflow's
    /// final result is the tool result.
    Tool {
        name: String,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_schema: Option<serde_json::Value>,
    },
}

/// Verification scheme for a generic webhook trigger. Reuses the
/// connection model — no separate credential store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebhookAuth {
    /// No verification (use only when the source is the public
    /// internet AND the workflow side-effects are safe to trigger).
    #[default]
    None,
    /// HMAC signature in a named header. Secret material comes from
    /// the referenced connection's auth field.
    HmacHeader {
        /// Header name carrying the signature, e.g. `X-Hub-Signature-256`.
        header: String,
        /// Connection whose secret material verifies the header.
        connection_id: uuid::Uuid,
    },
    /// Bearer token in `Authorization`. Token comes from the
    /// referenced connection.
    BearerToken { connection_id: uuid::Uuid },
}

/// How the webhook HTTP response is shaped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebhookResponse {
    /// 202 returned immediately; the workflow runs async.
    Ack,
    /// Wait for the workflow to run a `RespondToTrigger` step and
    /// return its body; times out after `timeout_secs`.
    Sync {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
}

impl Default for WebhookResponse {
    fn default() -> Self {
        Self::Ack
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_trigger_round_trips() {
        let json = serde_json::json!({
            "type": "slash",
            "name": "/join",
            "aliases": ["/continue"],
            "channels": ["telegram"],
            "args": ["code"]
        });
        let t: WorkflowTrigger = serde_json::from_value(json.clone()).unwrap();
        assert!(matches!(t, WorkflowTrigger::Slash { .. }));
        assert_eq!(serde_json::to_value(&t).unwrap(), json);
    }

    #[test]
    fn manual_trigger_round_trips() {
        let json = serde_json::json!({"type": "manual"});
        let t: WorkflowTrigger = serde_json::from_value(json.clone()).unwrap();
        assert!(matches!(t, WorkflowTrigger::Manual));
        assert_eq!(serde_json::to_value(&t).unwrap(), json);
    }

    #[test]
    fn webhook_trigger_round_trips() {
        let wt = WorkflowTrigger::Webhook {
            path: "github".into(),
            methods: vec!["POST".into()],
            auth: WebhookAuth::HmacHeader {
                header: "X-Hub-Signature-256".into(),
                connection_id: uuid::Uuid::new_v4(),
            },
            response: WebhookResponse::Ack,
        };
        let json = serde_json::to_value(&wt).unwrap();
        let back: WorkflowTrigger = serde_json::from_value(json).unwrap();
        assert_eq!(back, wt);
    }

    #[test]
    fn tool_trigger_round_trips() {
        let wt = WorkflowTrigger::Tool {
            name: "summarize".into(),
            description: "summarize a document".into(),
            input_schema: Some(serde_json::json!({"type": "object"})),
        };
        let json = serde_json::to_value(&wt).unwrap();
        let back: WorkflowTrigger = serde_json::from_value(json).unwrap();
        assert_eq!(back, wt);
    }

    #[test]
    fn event_trigger_round_trips() {
        let wt = WorkflowTrigger::Event {
            topic: "user.signup".into(),
            filter: Some(serde_json::json!({"plan": "pro"})),
        };
        let json = serde_json::to_value(&wt).unwrap();
        let back: WorkflowTrigger = serde_json::from_value(json).unwrap();
        assert_eq!(back, wt);
    }
}
