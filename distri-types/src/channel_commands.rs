//! Channel-facing command types. A `WorkflowAgent` bound to a bot uses
//! these to declare slash commands, callback buttons, and reply
//! rendering. See `docs/specs/workflow-channel-commands.md`.

use crate::channels::ChannelProvider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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

/// Author-facing button template inside a `StepKind::Reply`. Label/url/
/// callback_data may contain `{...}` interpolation (resolved by the
/// Reply step executor against workflow context, and `{item.*}` when
/// used as a `button_template`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyButtonSpec {
    Url { label: String, url: String },
    WebApp { label: String, url: String },
    Callback { label: String, callback_data: String },
}

/// A fully-resolved button (no interpolation left). Crosses the
/// workflow-executor → gateway boundary inside `AgentEventType::ChannelReply`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelButton {
    Url { label: String, url: String },
    WebApp { label: String, url: String },
    Callback { label: String, callback_data: String },
}

/// A fully-resolved channel reply emitted by a `StepKind::Reply` step.
/// `buttons` is rows of buttons (outer = rows top-to-bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelReply {
    pub text: String,
    #[serde(default)]
    pub buttons: Vec<Vec<ChannelButton>>,
}

/// Channel chrome for a workflow-agent bot (presentation only — per-
/// command behavior lives in entry points). All fields optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ChannelBindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram: Option<TelegramBinding>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TelegramBinding {
    /// In-chat persistent menu button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_button: Option<MenuButton>,
    /// Base URL prepended to relative WebApp button URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_app_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MenuButton {
    pub label: String,
    pub url: String,
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

    #[test]
    fn reply_button_spec_kinds_round_trip() {
        for json in [
            serde_json::json!({"kind":"url","label":"Docs","url":"https://d.dev"}),
            serde_json::json!({"kind":"web_app","label":"Open","url":"https://a.app"}),
            serde_json::json!({"kind":"callback","label":"Pick","callback_data":"wf:x"}),
        ] {
            let b: ReplyButtonSpec = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&b).unwrap(), json);
        }
    }

    #[test]
    fn channel_bindings_defaults_are_none() {
        let b: ChannelBindings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(b.telegram.is_none());
    }

    #[test]
    fn telegram_menu_button_round_trips() {
        let json = serde_json::json!({
            "telegram": {
                "menu_button": {"label":"Open","url":"https://a.app/learn"},
                "web_app_base": "https://a.app"
            }
        });
        let b: ChannelBindings = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(b.telegram.as_ref().unwrap().menu_button.as_ref().unwrap().label, "Open");
        assert_eq!(serde_json::to_value(&b).unwrap(), json);
    }

    #[test]
    fn channel_reply_holds_resolved_buttons() {
        let r = ChannelReply {
            text: "Your classes:".into(),
            buttons: vec![vec![ChannelButton::Callback {
                label: "Math".into(),
                callback_data: "wf:open_class:m1".into(),
            }]],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: ChannelReply = serde_json::from_value(v).unwrap();
        assert_eq!(back.text, "Your classes:");
        assert_eq!(back.buttons[0].len(), 1);
    }
}
