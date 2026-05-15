//! Channel-facing command types. A `WorkflowAgent` bound to a bot uses
//! these to declare slash commands, callback buttons, and reply
//! rendering. See `docs/specs/workflow-channel-commands.md`.

use crate::channels::ChannelProvider;
use serde::{Deserialize, Serialize};

/// How a channel user reaches a workflow entry point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelTrigger {
    /// A slash command. `args` names positional params, in order: for
    /// `args = ["code"]`, `/join ABC` sets `input.code = "ABC"`. The
    /// final named arg slurps all remaining whitespace-joined text.
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
    /// A callback-button tap. `callback_data` is `wf:<id>` or
    /// `wf:<id>:<value>`; `<value>` becomes `input[arg]`.
    Callback {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<String>,
    },
    /// The catch-all for non-command free text. At most one per
    /// workflow. Its entry point's `starts_at` step is normally an
    /// `AgentRun`.
    Message {},
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
        let t: ChannelTrigger = serde_json::from_value(json.clone()).unwrap();
        match &t {
            ChannelTrigger::Slash { name, aliases, channels, args } => {
                assert_eq!(name, "/join");
                assert_eq!(aliases, &vec!["/continue".to_string()]);
                assert_eq!(channels, &vec![ChannelProvider::Telegram]);
                assert_eq!(args, &vec!["code".to_string()]);
            }
            _ => panic!("expected Slash"),
        }
        assert_eq!(serde_json::to_value(&t).unwrap(), json);
    }

    #[test]
    fn slash_trigger_defaults_empty_vecs() {
        let t: ChannelTrigger =
            serde_json::from_value(serde_json::json!({"type":"slash","name":"/x"})).unwrap();
        match t {
            ChannelTrigger::Slash { aliases, channels, args, .. } => {
                assert!(aliases.is_empty() && channels.is_empty() && args.is_empty());
            }
            _ => panic!("expected Slash"),
        }
    }

    #[test]
    fn callback_and_message_round_trip() {
        let cb: ChannelTrigger = serde_json::from_value(
            serde_json::json!({"type":"callback","id":"open_class","arg":"class_id"}),
        )
        .unwrap();
        assert!(matches!(cb, ChannelTrigger::Callback { .. }));
        let msg: ChannelTrigger =
            serde_json::from_value(serde_json::json!({"type":"message"})).unwrap();
        assert!(matches!(msg, ChannelTrigger::Message {}));
    }
}
